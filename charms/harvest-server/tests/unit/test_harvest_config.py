import tomllib

import pytest

from harvest_config import (
    API_PATHS,
    DatabaseInfo,
    ServerSettings,
    build_database_url,
    render_server_toml,
    substitute_secrets,
)


def settings(**overrides):
    base = dict(
        database_url="postgres://u:p@10.0.0.1:5432/harvest",
        database_ca_file=None,
        pool_size=16,
        node_name="harvest-server-0",
        internal_url="http://10.0.0.11:8081",
        cluster_secret="cluster-secret",
        jwt_secret="jwt-secret",
        user_key_encryption_key="00" * 32,
        public_url="https://harvest.example.com",
        llm_providers='[[llm]]\nprovider = "gemini"\nmodel = "gemini-2.5-flash"\napi_key = ""\nuser_provided_key = true\n',
        allow_local_login=True,
        max_iterations=25,
        drain_timeout_secs=120,
        binary_path="/snap/harvest/current/bin/harvest-agent",
        oidc=None,
        google=None,
        extra_toml="",
    )
    base.update(overrides)
    return ServerSettings(**base)


def test_single_primary_endpoint_builds_a_read_write_url():
    url = build_database_url(DatabaseInfo(
        endpoints="10.0.0.1:5432", read_only_endpoints="", username="relation-7",
        password="s3cret", database="harvest", tls=False,
    ))
    assert url == "postgres://relation-7:s3cret@10.0.0.1:5432/harvest?target_session_attrs=read-write&connect_timeout=5"


def test_replicas_are_listed_after_the_primary_so_failover_needs_no_restart():
    url = build_database_url(DatabaseInfo(
        endpoints="10.0.0.1:5432", read_only_endpoints="10.0.0.2:5432,10.0.0.3:5432",
        username="u", password="p", database="harvest", tls=False,
    ))
    assert url.startswith("postgres://u:p@10.0.0.1:5432,10.0.0.2:5432,10.0.0.3:5432/harvest?")
    assert "target_session_attrs=read-write" in url


def test_duplicate_hosts_are_not_repeated():
    url = build_database_url(DatabaseInfo(
        endpoints="10.0.0.1:5432", read_only_endpoints="10.0.0.1:5432,10.0.0.2:5432",
        username="u", password="p", database="harvest", tls=False,
    ))
    assert "10.0.0.1:5432,10.0.0.2:5432/" in url
    assert url.count("10.0.0.1") == 1


def test_tls_requires_ssl():
    url = build_database_url(DatabaseInfo(
        endpoints="10.0.0.1:5432", read_only_endpoints="", username="u", password="p",
        database="harvest", tls=True,
    ))
    assert "sslmode=require" in url


def test_credentials_are_percent_encoded():
    url = build_database_url(DatabaseInfo(
        endpoints="h:5432", read_only_endpoints="", username="u@x", password="p/w:d?#",
        database="harvest", tls=False,
    ))
    assert url.startswith("postgres://u%40x:p%2Fw%3Ad%3F%23@h:5432/")


def test_secret_placeholders_are_replaced_and_unknown_ones_rejected():
    rendered = substitute_secrets('api_key = "${secret:openai}"', {"openai": "sk-1"})
    assert rendered == 'api_key = "sk-1"'
    with pytest.raises(KeyError):
        substitute_secrets('api_key = "${secret:missing}"', {})


def test_rendered_config_is_valid_toml_with_cluster_and_database_settings():
    parsed = tomllib.loads(render_server_toml(settings()))
    assert parsed["server"] == {"host": "0.0.0.0", "port": 8080}
    assert parsed["database"]["url"].startswith("postgres://")
    assert parsed["database"]["pool_size"] == 16
    assert parsed["database"]["migrate_on_start"] is False
    assert parsed["cluster"]["internal_listen"] == "0.0.0.0:8081"
    assert parsed["cluster"]["internal_url"] == "http://10.0.0.11:8081"
    assert parsed["cluster"]["shared_secret"] == "cluster-secret"
    assert parsed["cluster"]["drain_timeout_secs"] == 120
    assert parsed["auth"]["jwt_secret"] == "jwt-secret"
    assert parsed["auth"]["public_url"] == "https://harvest.example.com"
    assert parsed["agents"]["public_url"] == "https://harvest.example.com"
    assert parsed["security"]["user_key_encryption_key"] == "00" * 32
    assert parsed["llm"][0]["provider"] == "gemini"
    assert parsed["ui"]["enable_docs"] is False


def test_database_ca_file_is_rendered_when_tls_is_used():
    parsed = tomllib.loads(render_server_toml(settings(database_ca_file="/var/snap/harvest/common/db-ca.pem")))
    assert parsed["database"]["ca_file"] == "/var/snap/harvest/common/db-ca.pem"


def test_oidc_and_google_sections_are_optional():
    parsed = tomllib.loads(render_server_toml(settings()))
    assert "oidc" not in parsed["auth"]
    parsed = tomllib.loads(render_server_toml(settings(
        oidc={"issuer_url": "https://login.ubuntu.com", "client_id": "harvest", "client_secret": "x",
              "redirect_uri": "https://harvest.example.com/auth/oidc/callback", "display_name": "Ubuntu One"},
        google={"client_id": "g", "client_secret": "gs", "redirect_uri": "https://harvest.example.com/auth/google/callback"},
    )))
    assert parsed["auth"]["oidc"]["display_name"] == "Ubuntu One"
    assert parsed["auth"]["google"]["client_id"] == "g"


def test_strings_with_quotes_are_escaped():
    parsed = tomllib.loads(render_server_toml(settings(jwt_secret='a"b\\c')))
    assert parsed["auth"]["jwt_secret"] == 'a"b\\c'


def test_extra_toml_is_appended_verbatim():
    parsed = tomllib.loads(render_server_toml(settings(extra_toml='[semantic]\nenabled = false\n')))
    assert parsed["semantic"]["enabled"] is False


def test_api_paths_cover_every_server_route_prefix():
    for prefix in ["/auth", "/projects", "/agent", "/agents", "/health", "/version", "/metrics", "/llm",
                   "/repositories", "/graph", "/conversations", "/admin", "/skills", "/artifacts"]:
        assert prefix in API_PATHS
    assert all(p.startswith("/") and not p.endswith("/") for p in API_PATHS)
