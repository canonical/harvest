use std::sync::Arc;
use std::time::Duration;

use crate::cluster::membership::{Announcement, Membership, MembershipError};

pub struct Subject {
    pub membership: Arc<dyn Membership>,
    pub ttl: Duration,
    pub evicts: bool,
}

fn announcement(node_id: &str, draining: bool) -> Announcement {
    Announcement {
        node_id: node_id.to_string(),
        internal_url: format!("http://{node_id}.internal:8081"),
        version: "test".into(),
        draining,
    }
}

async fn alive_ids(subject: &Subject) -> Vec<String> {
    subject
        .membership
        .alive(subject.ttl)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.node_id)
        .collect()
}

pub async fn an_announced_node_is_alive_with_its_details(subject: &Subject) {
    subject
        .membership
        .announce(&announcement("a", false))
        .await
        .unwrap();
    let members = subject.membership.alive(subject.ttl).await.unwrap();
    let a = members
        .iter()
        .find(|m| m.node_id == "a")
        .expect("a is alive");
    assert_eq!(a.internal_url, "http://a.internal:8081");
    assert!(!a.draining);
}

pub async fn draining_is_visible_to_everyone(subject: &Subject) {
    subject
        .membership
        .announce(&announcement("b", false))
        .await
        .unwrap();
    subject
        .membership
        .announce(&announcement("b", true))
        .await
        .unwrap();
    let members = subject.membership.alive(subject.ttl).await.unwrap();
    assert!(members.iter().any(|m| m.node_id == "b" && m.draining));
}

pub async fn a_withdrawn_node_is_gone(subject: &Subject) {
    subject
        .membership
        .announce(&announcement("c", false))
        .await
        .unwrap();
    subject.membership.withdraw("c").await.unwrap();
    assert!(!alive_ids(subject).await.contains(&"c".to_string()));
}

pub async fn a_silent_node_disappears_and_a_live_one_stays(subject: &Subject) {
    subject
        .membership
        .announce(&announcement("quiet", false))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + subject.ttl * 4;
    loop {
        subject
            .membership
            .announce(&announcement("chatty", false))
            .await
            .unwrap();
        let alive = alive_ids(subject).await;
        if !alive.contains(&"quiet".to_string()) {
            assert!(alive.contains(&"chatty".to_string()));
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "quiet never expired"
        );
        tokio::time::sleep(subject.ttl / 4).await;
    }
    if subject.evicts {
        let returned = subject
            .membership
            .announce(&announcement("quiet", false))
            .await;
        assert!(matches!(returned, Err(MembershipError::Evicted)));
    }
}

pub async fn run_all(subject: &Subject) {
    an_announced_node_is_alive_with_its_details(subject).await;
    draining_is_visible_to_everyone(subject).await;
    a_withdrawn_node_is_gone(subject).await;
    a_silent_node_disappears_and_a_live_one_stays(subject).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::postgres::PostgresMembership;
    use crate::cluster::quorium::QuoriumMembership;
    use quorium::{Quorium, QuoriumConfig, Tuning};

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn postgres_membership_conforms() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let subject = Subject {
            membership: PostgresMembership::new(Arc::new(test_db.db.clone())),
            ttl: Duration::from_millis(1_500),
            evicts: false,
        };
        run_all(&subject).await;
    }

    async fn quorium_server(dir: &std::path::Path) -> Arc<Quorium> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let quorium = Quorium::start(QuoriumConfig {
            data_dir: dir.to_path_buf(),
            advertise_url: url,
            heartbeat_interval: Duration::from_millis(50),
            election_timeout: Duration::from_millis(300),
            session_ttl: Duration::from_millis(800),
            http: reqwest::Client::new(),
            admit_without_token: false,
            tuning: Tuning::default(),
        })
        .await
        .unwrap();
        let router = quorium::api::router(Arc::clone(&quorium));
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        quorium
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn quorium_membership_conforms() {
        let dir = tempfile::tempdir().unwrap();
        let quorium = quorium_server(dir.path()).await;
        quorium.initialize().await.unwrap();
        let subject = Subject {
            membership: QuoriumMembership::new(Arc::clone(&quorium)),
            ttl: Duration::from_millis(800),
            evicts: true,
        };
        run_all(&subject).await;
        quorium.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn quorium_followers_see_members_registered_through_the_leader() {
        let dirs: Vec<_> = (0..2).map(|_| tempfile::tempdir().unwrap()).collect();
        let leader = quorium_server(dirs[0].path()).await;
        leader.initialize().await.unwrap();
        let token = leader.issue_token(Duration::from_secs(60)).await.unwrap();
        let follower = quorium_server(dirs[1].path()).await;
        follower.join(&token, Duration::from_secs(5)).await.unwrap();
        let through_leader = QuoriumMembership::new(Arc::clone(&leader));
        let through_follower = QuoriumMembership::new(Arc::clone(&follower));
        through_leader
            .announce(&announcement("x", false))
            .await
            .unwrap();
        let seen = through_follower
            .alive(Duration::from_millis(800))
            .await
            .unwrap();
        assert!(
            seen.iter().any(|m| m.node_id == "x"),
            "linearizable reads follow the leader"
        );
        follower.shutdown().await;
        leader.shutdown().await;
    }
}
