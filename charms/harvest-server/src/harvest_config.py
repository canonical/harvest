import re
from dataclasses import dataclass
from typing import Mapping, Optional
from urllib.parse import quote

SERVER_PORT = 8080
INTERNAL_PORT = 8081

API_PATHS = [
    "/admin",
    "/agent",
    "/agents",
    "/artifacts",
    "/auth",
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
]

_SECRET_PLACEHOLDER = re.compile(r"\$\{secret:([A-Za-z0-9_.-]+)\}")


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


def _hosts(*lists: str) -> list:
    seen = []
    for value in lists:
        for host in (value or "").split(","):
            host = host.strip()
            if host and host not in seen:
                seen.append(host)
    return seen


def build_database_url(info: DatabaseInfo) -> str:
    hosts = ",".join(_hosts(info.endpoints, info.read_only_endpoints))
    params = ["target_session_attrs=read-write", "connect_timeout=5"]
    if info.tls:
        params.append("sslmode=require")
    user = quote(info.username, safe="")
    password = quote(info.password, safe="")
    return f"postgres://{user}:{password}@{hosts}/{quote(info.database, safe='')}?{'&'.join(params)}"


def substitute_secrets(text: str, secrets: Mapping[str, str]) -> str:
    def replace(match):
        return secrets[match.group(1)]

    return _SECRET_PLACEHOLDER.sub(replace, text)


def _quote(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
    return f'"{escaped}"'


def _bool(value: bool) -> str:
    return "true" if value else "false"


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
    lines += [
        "",
        "[cluster]",
        f"node_name = {_quote(s.node_name)}",
        f'internal_listen = "0.0.0.0:{INTERNAL_PORT}"',
        f"internal_url = {_quote(s.internal_url)}",
        f"shared_secret = {_quote(s.cluster_secret)}",
        f"drain_timeout_secs = {s.drain_timeout_secs}",
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
