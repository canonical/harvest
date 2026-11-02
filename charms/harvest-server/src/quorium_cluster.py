import hashlib
from dataclasses import dataclass
from typing import Iterable, Mapping, Optional

PRIMARY = "cluster-secret"
EXTRA = "cluster-secret-extra"
PHASE = "rotation-phase"
STAGED = "staged"
COMMITTED = "committed"


class RotationInProgress(Exception):
    pass


@dataclass(frozen=True)
class UnitReport:
    primary_digest: str
    accepted_digests: frozenset


def digest(secret: str) -> str:
    return hashlib.sha256(secret.encode()).hexdigest()[:16]


def report_from(data: Mapping[str, str]) -> UnitReport:
    accepted = {d for d in data.get("cluster-accepted-digests", "").split(",") if d}
    return UnitReport(data.get("cluster-secret-digest", ""), frozenset(accepted))


def report_fields(primary: str, accepted: Iterable[str]) -> dict:
    return {
        "cluster-secret-digest": digest(primary),
        "cluster-accepted-digests": ",".join(sorted(digest(s) for s in accepted)),
    }


def rendered_secrets(content: Mapping[str, str]) -> tuple:
    extra = content.get(EXTRA)
    return content[PRIMARY], tuple([extra] if extra else [])


def stage_rotation(content: Mapping[str, str], new_secret: str) -> dict:
    if content.get(PHASE):
        raise RotationInProgress(f"a rotation is already {content[PHASE]}")
    return {**content, EXTRA: new_secret, PHASE: STAGED}


def advance_rotation(content: Mapping[str, str], reports: Iterable[UnitReport]) -> Optional[dict]:
    reports = list(reports)
    phase = content.get(PHASE, "")
    if not reports or not phase:
        return None
    if phase == STAGED:
        pending = digest(content[EXTRA])
        if all(pending in r.accepted_digests or r.primary_digest == pending for r in reports):
            return {**content, PRIMARY: content[EXTRA], EXTRA: content[PRIMARY], PHASE: COMMITTED}
        return None
    if phase == COMMITTED:
        current = digest(content[PRIMARY])
        if all(r.primary_digest == current for r in reports):
            return {k: v for k, v in content.items() if k not in (EXTRA, PHASE)}
    return None


def join_plan(unit: str, seed: Optional[str], peers: Mapping[str, str]) -> tuple:
    if seed is None:
        return None, ()
    if unit == seed:
        return "init", ()
    return "join", tuple(sorted(url for name, url in peers.items() if name != unit and url))


def stale_servers(servers: Iterable[Mapping], live_urls: Iterable[str]) -> list:
    live = {url.rstrip("/") for url in live_urls}
    return sorted(
        s["address"] for s in servers if s.get("address", "").rstrip("/") not in live
    )


def restart_is_safe(status: Mapping, own_url: str) -> bool:
    servers = (status.get("quorium") or {}).get("servers") or []
    voters = [s.get("address", "").rstrip("/") for s in servers if s.get("voter")]
    own = own_url.rstrip("/")
    if len(voters) <= 1 or own not in voters:
        return True
    live = {m.get("internal_url", "").rstrip("/") for m in status.get("members", [])}
    healthy = sum(1 for address in voters if address in live)
    return healthy - 1 >= len(voters) // 2 + 1
