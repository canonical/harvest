from dataclasses import dataclass, field
from typing import Mapping, Optional, Sequence
from urllib.parse import quote

SERVER_PORT = 8080
INTERNAL_PORT = 8081
QUORIUM_DATA_DIR = "/var/snap/harvest/common/quorium"
BACKENDS = ("postgres", "quorium")

API_PATHS = [
    "/admin",
    "/agent",
    "/agents",
    "/artifacts",
    "/auth",
    "/api-keys",
    "/chat-layouts",
    "/conversations",
    "/docs",
    "/graph",
    "/groups",
    "/health",
    "/llm",
    "/machines",
    "/metrics",
    "/projects",
    "/query",
    "/repositories",
    "/skills",
    "/templates",
    "/tool-description",
    "/version",
    "/v1",
]

@dataclass(frozen=True)
class DatabaseInfo:
    endpoints: str
    read_only_endpoints: str
    username: str
    password: str
    database: str
    tls: bool


@dataclass(frozen=True)
class ServerSettings:
    database_url: str
    database_ca_file: Optional[str]
    pool_size: int
    node_name: str
    internal_url: str
    cluster_secret: str
    jwt_secret: str
    user_key_encryption_key: str
    public_url: Optional[str]
    llm_providers: str
    allow_local_login: bool
    max_iterations: int
    drain_timeout_secs: int
    binary_path: str
    oidc: Optional[Mapping[str, str]]
    google: Optional[Mapping[str, str]]
    extra_toml: str
    backend: str = "postgres"
    bootstrap: Optional[str] = None
    join_addresses: Sequence[str] = field(default_factory=tuple)
    accepted_secrets: Sequence[str] = field(default_factory=tuple)


def _hosts(*lists: str) -> list:
    return sorted({host.strip() for value in lists for host in (value or "").split(",") if host.strip()})


def database_hosts(info: DatabaseInfo) -> list:
    return _hosts(info.endpoints, info.read_only_endpoints)


def build_database_url(info: DatabaseInfo) -> str:
    hosts = ",".join(database_hosts(info))
    params = ["target_session_attrs=read-write", "connect_timeout=5"]
    if info.tls:
        params.append("sslmode=require")
    user = quote(info.username, safe="")
    password = quote(info.password, safe="")
    return f"postgres://{user}:{password}@{hosts}/{quote(info.database, safe='')}?{'&'.join(params)}"


def _quote(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
    return f'"{escaped}"'


def _bool(value: bool) -> str:
    return "true" if value else "false"


def _list(values: Sequence[str]) -> str:
    return "[" + ", ".join(_quote(v) for v in values) + "]"


def _cluster_lines(s: ServerSettings) -> list:
    lines = [
        "[cluster]",
        f"node_name = {_quote(s.node_name)}",
        f'internal_listen = "0.0.0.0:{INTERNAL_PORT}"',
        f"internal_url = {_quote(s.internal_url)}",
        f"shared_secret = {_quote(s.cluster_secret)}",
        f"drain_timeout_secs = {s.drain_timeout_secs}",
    ]
    if s.accepted_secrets:
        lines.append(f"accepted_secrets = {_list(s.accepted_secrets)}")
    if s.backend != "quorium":
        return lines
    lines += [
        'backend = "quorium"',
        f"data_dir = {_quote(QUORIUM_DATA_DIR)}",
        "heartbeat_interval_ms = 1000",
        "node_timeout_ms = 5000",
        "election_timeout_ms = 1000",
    ]
    if s.bootstrap:
        lines.append(f"bootstrap = {_quote(s.bootstrap)}")
    if s.join_addresses:
        lines.append(f"join_addresses = {_list(s.join_addresses)}")
    return lines


def render_server_toml(s: ServerSettings) -> str:
    lines = [
        "[server]",
        'host = "0.0.0.0"',
        f"port = {SERVER_PORT}",
        "",
        "[database]",
        f"url = {_quote(s.database_url)}",
        f"pool_size = {s.pool_size}",
        "migrate_on_start = false",
    ]
    if s.database_ca_file:
        lines.append(f"ca_file = {_quote(s.database_ca_file)}")
    lines += ["", *_cluster_lines(s)]
    lines += [
        "",
        "[agent]",
        f"max_iterations = {s.max_iterations}",
        "",
        "[auth]",
        f"jwt_secret = {_quote(s.jwt_secret)}",
        f"allow_local_login = {_bool(s.allow_local_login)}",
    ]
    if s.public_url:
        lines.append(f"public_url = {_quote(s.public_url)}")
    if s.google:
        lines += [
            "",
            "[auth.google]",
            f"client_id = {_quote(s.google['client_id'])}",
            f"client_secret = {_quote(s.google['client_secret'])}",
            f"redirect_uri = {_quote(s.google['redirect_uri'])}",
        ]
    if s.oidc:
        lines += [
            "",
            "[auth.oidc]",
            f"issuer_url = {_quote(s.oidc['issuer_url'])}",
            f"client_id = {_quote(s.oidc['client_id'])}",
            f"client_secret = {_quote(s.oidc['client_secret'])}",
            f"redirect_uri = {_quote(s.oidc['redirect_uri'])}",
        ]
        if s.oidc.get("display_name"):
            lines.append(f"display_name = {_quote(s.oidc['display_name'])}")
    lines += [
        "",
        "[security]",
        f"user_key_encryption_key = {_quote(s.user_key_encryption_key)}",
        "",
        "[agents]",
        f"binary_path = {_quote(s.binary_path)}",
    ]
    if s.public_url:
        lines.append(f"public_url = {_quote(s.public_url)}")
    lines += [
        "",
        "[ui]",
        "enable_docs = false",
        "",
        s.llm_providers.strip(),
        "",
    ]
    if s.extra_toml.strip():
        lines += [s.extra_toml.strip(), ""]
    return "\n".join(lines)
