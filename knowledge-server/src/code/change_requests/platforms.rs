use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::{RequestBuilder, StatusCode};
use serde_json::{json, Value};

use crate::credentials::platform::Platform;
use crate::credentials::remote::RemoteUrl;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Open,
    Merged,
    Closed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }
}

pub struct Proposal<'a> {
    pub branch: &'a str,
    pub base: &'a str,
    pub title: &'a str,
    pub description: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushTarget {
    pub url: String,
    pub destination: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub number: String,
    pub url: String,
}

#[async_trait]
pub trait GitPlatform: Send + Sync {
    fn push_target(&self, proposal: &Proposal<'_>) -> PushTarget;
    async fn open(&self, proposal: &Proposal<'_>, push_messages: &str) -> Result<Opened>;
    async fn status(&self, number: &str) -> Result<Status>;
}

pub struct Account {
    pub username: String,
    pub secret: String,
}

pub struct Repository {
    pub url: String,
    pub remote: RemoteUrl,
}

pub fn account_host(platform: Platform, remote: &RemoteUrl) -> String {
    match platform {
        Platform::Gerrit => review_host(&remote.host),
        Platform::Launchpad => "launchpad.net".into(),
        _ => remote.host.clone(),
    }
}

pub fn review_host(host: &str) -> String {
    match host {
        "opendev.org" => "review.opendev.org".into(),
        other => other.to_string(),
    }
}

pub fn build(platform: Platform, repo: Repository, account: Account) -> Box<dyn GitPlatform> {
    let http = reqwest::Client::builder()
        .user_agent("harvest")
        .build()
        .unwrap_or_default();
    let ctx = Connection { repo, account, http };
    match platform {
        Platform::Github => Box::new(Github(ctx)),
        Platform::Gitlab => Box::new(Gitlab(ctx)),
        Platform::Gitea => Box::new(Gitea(ctx)),
        Platform::Gerrit => Box::new(Gerrit(ctx)),
        Platform::Launchpad => Box::new(Launchpad(ctx)),
    }
}

struct Connection {
    repo: Repository,
    account: Account,
    http: reqwest::Client,
}

impl Connection {
    fn path(&self) -> &str {
        &self.repo.remote.path
    }

    fn host(&self) -> &str {
        &self.repo.remote.host
    }

    fn same_repo_target(&self, branch: &str) -> PushTarget {
        PushTarget {
            url: self.repo.url.clone(),
            destination: format!("refs/heads/{branch}"),
        }
    }
}

async fn json_of(request: RequestBuilder) -> Result<(StatusCode, Value)> {
    let response = request.send().await.context("contacting the platform")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let body = serde_json::from_str(text.trim_start_matches(")]}'")).unwrap_or(Value::String(text));
    Ok((status, body))
}

fn expect_ok(status: StatusCode, body: &Value, what: &str) -> Result<()> {
    if !status.is_success() {
        let message = body["message"].as_str().map(String::from).unwrap_or_else(|| body.to_string());
        bail!("{what} failed ({status}): {}", super::super::images::tail(&message, 500));
    }
    Ok(())
}

fn number_text(value: &Value) -> Option<String> {
    value.as_u64().map(|n| n.to_string()).or_else(|| value.as_str().map(String::from))
}

pub fn encode_component(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

struct Github(Connection);

impl Github {
    fn api(&self) -> String {
        match self.0.host() {
            "github.com" => format!("https://api.github.com/repos/{}", self.0.path()),
            host => format!("https://{host}/api/v3/repos/{}", self.0.path()),
        }
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .bearer_auth(&self.0.account.secret)
            .header("Accept", "application/vnd.github+json")
    }
}

#[async_trait]
impl GitPlatform for Github {
    fn push_target(&self, proposal: &Proposal<'_>) -> PushTarget {
        self.0.same_repo_target(proposal.branch)
    }

    async fn open(&self, proposal: &Proposal<'_>, _: &str) -> Result<Opened> {
        let body = json!({
            "title": proposal.title, "body": proposal.description,
            "head": proposal.branch, "base": proposal.base,
        });
        let (status, created) = json_of(self.auth(self.0.http.post(format!("{}/pulls", self.api())).json(&body))).await?;
        let pull = if status == StatusCode::UNPROCESSABLE_ENTITY {
            let owner = self.0.path().split('/').next().unwrap_or_default();
            let (status, list) = json_of(self.auth(
                self.0
                    .http
                    .get(format!("{}/pulls", self.api()))
                    .query(&[("state", "open"), ("head", &format!("{owner}:{}", proposal.branch))]),
            ))
            .await?;
            expect_ok(status, &list, "finding the pull request")?;
            list.get(0).cloned().context(format!("opening the pull request failed: {created}"))?
        } else {
            expect_ok(status, &created, "opening the pull request")?;
            created
        };
        Ok(Opened {
            number: number_text(&pull["number"]).context("no pull request number")?,
            url: pull["html_url"].as_str().unwrap_or_default().to_string(),
        })
    }

    async fn status(&self, number: &str) -> Result<Status> {
        let (status, pull) = json_of(self.auth(self.0.http.get(format!("{}/pulls/{number}", self.api())))).await?;
        expect_ok(status, &pull, "reading the pull request")?;
        Ok(match (pull["merged"].as_bool(), pull["state"].as_str()) {
            (Some(true), _) => Status::Merged,
            (_, Some("closed")) => Status::Closed,
            _ => Status::Open,
        })
    }
}

struct Gitlab(Connection);

impl Gitlab {
    fn api(&self) -> String {
        format!("https://{}/api/v4/projects/{}", self.0.host(), encode_component(self.0.path()))
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        request.header("PRIVATE-TOKEN", &self.0.account.secret)
    }
}

#[async_trait]
impl GitPlatform for Gitlab {
    fn push_target(&self, proposal: &Proposal<'_>) -> PushTarget {
        self.0.same_repo_target(proposal.branch)
    }

    async fn open(&self, proposal: &Proposal<'_>, _: &str) -> Result<Opened> {
        let body = json!({
            "source_branch": proposal.branch, "target_branch": proposal.base,
            "title": proposal.title, "description": proposal.description,
        });
        let (status, created) =
            json_of(self.auth(self.0.http.post(format!("{}/merge_requests", self.api())).json(&body))).await?;
        let request = if status == StatusCode::CONFLICT {
            let (status, list) = json_of(self.auth(
                self.0
                    .http
                    .get(format!("{}/merge_requests", self.api()))
                    .query(&[("state", "opened"), ("source_branch", proposal.branch)]),
            ))
            .await?;
            expect_ok(status, &list, "finding the merge request")?;
            list.get(0).cloned().context("the existing merge request was not found")?
        } else {
            expect_ok(status, &created, "opening the merge request")?;
            created
        };
        Ok(Opened {
            number: number_text(&request["iid"]).context("no merge request number")?,
            url: request["web_url"].as_str().unwrap_or_default().to_string(),
        })
    }

    async fn status(&self, number: &str) -> Result<Status> {
        let (status, request) =
            json_of(self.auth(self.0.http.get(format!("{}/merge_requests/{number}", self.api())))).await?;
        expect_ok(status, &request, "reading the merge request")?;
        Ok(match request["state"].as_str() {
            Some("merged") => Status::Merged,
            Some("closed") => Status::Closed,
            _ => Status::Open,
        })
    }
}

struct Gitea(Connection);

impl Gitea {
    fn api(&self) -> String {
        format!("https://{}/api/v1/repos/{}", self.0.host(), self.0.path())
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        request.header("Authorization", format!("token {}", self.0.account.secret))
    }
}

#[async_trait]
impl GitPlatform for Gitea {
    fn push_target(&self, proposal: &Proposal<'_>) -> PushTarget {
        self.0.same_repo_target(proposal.branch)
    }

    async fn open(&self, proposal: &Proposal<'_>, _: &str) -> Result<Opened> {
        let body = json!({
            "head": proposal.branch, "base": proposal.base,
            "title": proposal.title, "body": proposal.description,
        });
        let (status, created) = json_of(self.auth(self.0.http.post(format!("{}/pulls", self.api())).json(&body))).await?;
        let pull = if status == StatusCode::CONFLICT || status == StatusCode::UNPROCESSABLE_ENTITY {
            let (status, list) = json_of(self.auth(
                self.0.http.get(format!("{}/pulls", self.api())).query(&[("state", "open")]),
            ))
            .await?;
            expect_ok(status, &list, "finding the pull request")?;
            list.as_array()
                .and_then(|pulls| pulls.iter().find(|p| p["head"]["ref"].as_str() == Some(proposal.branch)))
                .cloned()
                .context(format!("opening the pull request failed: {created}"))?
        } else {
            expect_ok(status, &created, "opening the pull request")?;
            created
        };
        Ok(Opened {
            number: number_text(&pull["number"]).context("no pull request number")?,
            url: pull["html_url"].as_str().unwrap_or_default().to_string(),
        })
    }

    async fn status(&self, number: &str) -> Result<Status> {
        let (status, pull) = json_of(self.auth(self.0.http.get(format!("{}/pulls/{number}", self.api())))).await?;
        expect_ok(status, &pull, "reading the pull request")?;
        Ok(match (pull["merged"].as_bool(), pull["state"].as_str()) {
            (Some(true), _) => Status::Merged,
            (_, Some("closed")) => Status::Closed,
            _ => Status::Open,
        })
    }
}

struct Gerrit(Connection);

pub fn gerrit_change(messages: &str) -> Option<Opened> {
    messages.split_whitespace().find_map(|token| {
        let (_, after) = token.split_once("/+/")?;
        let number: String = after.chars().take_while(char::is_ascii_digit).collect();
        (token.starts_with("http") && !number.is_empty()).then(|| Opened {
            number,
            url: token.to_string(),
        })
    })
}

pub fn gerrit_topic(branch: &str) -> String {
    branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

#[async_trait]
impl GitPlatform for Gerrit {
    fn push_target(&self, proposal: &Proposal<'_>) -> PushTarget {
        PushTarget {
            url: format!("https://{}/{}", review_host(self.0.host()), self.0.path()),
            destination: format!("refs/for/{}%topic={}", proposal.base, gerrit_topic(proposal.branch)),
        }
    }

    async fn open(&self, _: &Proposal<'_>, push_messages: &str) -> Result<Opened> {
        gerrit_change(push_messages).context("Gerrit accepted the push but did not report a change")
    }

    async fn status(&self, number: &str) -> Result<Status> {
        let url = format!("https://{}/a/changes/{number}", review_host(self.0.host()));
        let request = self
            .0
            .http
            .get(url)
            .basic_auth(&self.0.account.username, Some(&self.0.account.secret));
        let (status, change) = json_of(request).await?;
        expect_ok(status, &change, "reading the change")?;
        Ok(match change["status"].as_str() {
            Some("MERGED") => Status::Merged,
            Some("ABANDONED") => Status::Closed,
            _ => Status::Open,
        })
    }
}

struct Launchpad(Connection);

const LAUNCHPAD_API: &str = "https://api.launchpad.net/devel";

pub fn launchpad_project(path: &str) -> &str {
    match path.strip_prefix('~') {
        Some(rest) => rest.split('/').nth(1).unwrap_or(rest),
        None => path.split('/').next().unwrap_or(path),
    }
}

pub fn launchpad_web_url(api_link: &str) -> String {
    api_link.replacen(LAUNCHPAD_API, "https://code.launchpad.net", 1)
}

impl Launchpad {
    fn personal_path(&self) -> String {
        format!("~{}/{}", self.0.account.username, launchpad_project(self.0.path()))
    }

    fn oauth(&self, request: RequestBuilder) -> RequestBuilder {
        let (token, secret) = self.0.account.secret.split_once(':').unwrap_or((&self.0.account.secret, ""));
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let header = format!(
            "OAuth realm=\"https://api.launchpad.net/\", oauth_consumer_key=\"harvest\", oauth_token=\"{token}\", \
             oauth_signature_method=\"PLAINTEXT\", oauth_signature=\"&{secret}\", oauth_timestamp=\"{now}\", \
             oauth_nonce=\"{}\", oauth_version=\"1.0\"",
            uuid::Uuid::new_v4().simple()
        );
        request.header("Authorization", header)
    }

    async fn repository_link(&self, path: &str) -> Result<String> {
        let request = self
            .0
            .http
            .get(format!("{LAUNCHPAD_API}/+git"))
            .query(&[("ws.op", "getByPath"), ("path", path)]);
        let (status, repo) = json_of(self.oauth(request)).await?;
        expect_ok(status, &repo, "finding the Launchpad repository")?;
        repo["self_link"]
            .as_str()
            .map(String::from)
            .with_context(|| format!("Launchpad has no repository at {path}"))
    }
}

#[async_trait]
impl GitPlatform for Launchpad {
    fn push_target(&self, proposal: &Proposal<'_>) -> PushTarget {
        PushTarget {
            url: format!("git+ssh://{}@git.launchpad.net/{}", self.0.account.username, self.personal_path()),
            destination: format!("refs/heads/{}", proposal.branch),
        }
    }

    async fn open(&self, proposal: &Proposal<'_>, _: &str) -> Result<Opened> {
        let source = self.repository_link(&self.personal_path()).await?;
        let target = self.repository_link(self.0.path()).await?;
        let form = [
            ("ws.op", "createMergeProposal".to_string()),
            ("merge_target", format!("{target}/+ref/{}", proposal.base)),
            ("commit_message", proposal.title.to_string()),
            ("description", proposal.description.to_string()),
        ];
        let request = self
            .0
            .http
            .post(format!("{source}/+ref/{}", proposal.branch))
            .form(&form);
        let response = self.oauth(request).send().await.context("contacting Launchpad")?;
        let status = response.status();
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let body = response.text().await.unwrap_or_default();
        let link = location
            .or_else(|| serde_json::from_str::<Value>(&body).ok()?["self_link"].as_str().map(String::from))
            .with_context(|| format!("creating the merge proposal failed ({status}): {body}"))?;
        Ok(Opened {
            url: launchpad_web_url(&link),
            number: link,
        })
    }

    async fn status(&self, number: &str) -> Result<Status> {
        let (status, proposal) = json_of(self.oauth(self.0.http.get(number))).await?;
        expect_ok(status, &proposal, "reading the merge proposal")?;
        Ok(match proposal["queue_status"].as_str() {
            Some("Merged") => Status::Merged,
            Some("Rejected" | "Superseded") => Status::Closed,
            _ => Status::Open,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform(kind: Platform, url: &str) -> Box<dyn GitPlatform> {
        build(
            kind,
            Repository { url: url.into(), remote: RemoteUrl::parse(url).unwrap() },
            Account { username: "ana".into(), secret: "s".into() },
        )
    }

    fn proposal() -> Proposal<'static> {
        Proposal { branch: "feat/login", base: "main", title: "t", description: "d" }
    }

    #[test]
    fn hosted_platforms_push_the_branch_to_the_repository_itself() {
        for kind in [Platform::Github, Platform::Gitlab, Platform::Gitea] {
            let target = platform(kind, "https://example.org/team/app").push_target(&proposal());
            assert_eq!(target.url, "https://example.org/team/app");
            assert_eq!(target.destination, "refs/heads/feat/login");
        }
    }

    #[test]
    fn gerrit_pushes_for_review_on_the_review_host() {
        let target = platform(Platform::Gerrit, "https://opendev.org/openstack/cinder").push_target(&proposal());
        assert_eq!(target.url, "https://review.opendev.org/openstack/cinder");
        assert_eq!(target.destination, "refs/for/main%topic=feat-login");
    }

    #[test]
    fn launchpad_pushes_to_the_personal_repository_of_the_project() {
        let target = platform(Platform::Launchpad, "https://git.launchpad.net/cloud-init").push_target(&proposal());
        assert_eq!(target.url, "git+ssh://ana@git.launchpad.net/~ana/cloud-init");
        assert_eq!(launchpad_project("~canonical/maas/+git/maas"), "maas");
    }

    #[test]
    fn the_gerrit_change_is_read_from_the_push_messages() {
        let messages = "remote: Processing changes: new: 1\nremote:   https://review.opendev.org/c/openstack/cinder/+/912345 Fix login [NEW]\n";
        let change = gerrit_change(messages).unwrap();
        assert_eq!(change.number, "912345");
        assert_eq!(change.url, "https://review.opendev.org/c/openstack/cinder/+/912345");
        assert!(gerrit_change("remote: no new changes").is_none());
    }

    #[test]
    fn gitlab_project_paths_are_encoded_as_one_segment() {
        assert_eq!(encode_component("group/sub/app"), "group%2Fsub%2Fapp");
    }

    #[test]
    fn launchpad_api_links_become_web_links() {
        assert_eq!(
            launchpad_web_url("https://api.launchpad.net/devel/~ana/x/+git/x/+merge/7"),
            "https://code.launchpad.net/~ana/x/+git/x/+merge/7"
        );
    }

    #[test]
    fn accounts_for_gerrit_and_launchpad_live_on_their_api_hosts() {
        let opendev = RemoteUrl::parse("https://opendev.org/openstack/nova").unwrap();
        assert_eq!(account_host(Platform::Gerrit, &opendev), "review.opendev.org");
        let lp = RemoteUrl::parse("https://git.launchpad.net/nova").unwrap();
        assert_eq!(account_host(Platform::Launchpad, &lp), "launchpad.net");
        let gh = RemoteUrl::parse("https://github.com/a/b").unwrap();
        assert_eq!(account_host(Platform::Github, &gh), "github.com");
    }
}
