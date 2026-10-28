import pytest

from quorium_cluster import (
    COMMITTED,
    EXTRA,
    PHASE,
    PRIMARY,
    STAGED,
    RotationInProgress,
    UnitReport,
    advance_rotation,
    digest,
    join_plan,
    rendered_secrets,
    report_fields,
    report_from,
    restart_is_safe,
    stage_rotation,
    stale_servers,
)


def reports(*pairs):
    return [UnitReport(primary, frozenset(accepted)) for primary, accepted in pairs]


def test_the_seed_initializes_and_others_join_every_peer():
    peers = {"h/0": "http://a:8081", "h/1": "http://b:8081", "h/2": "http://c:8081"}
    assert join_plan("h/0", "h/0", peers) == ("init", ())
    assert join_plan("h/1", "h/0", peers) == ("join", ("http://a:8081", "http://c:8081"))
    assert join_plan("h/1", None, peers) == (None, ())


def test_servers_without_a_unit_are_stale():
    servers = [{"address": "http://a:8081"}, {"address": "http://gone:8081/"}]
    assert stale_servers(servers, ["http://a:8081/"]) == ["http://gone:8081/"]


def test_rendering_follows_the_rotation_phase():
    assert rendered_secrets({PRIMARY: "old"}) == ("old", ())
    assert rendered_secrets({PRIMARY: "old", EXTRA: "new", PHASE: STAGED}) == ("old", ("new",))


def test_a_rotation_cannot_start_twice():
    staged = stage_rotation({PRIMARY: "old"}, "new")
    assert staged[EXTRA] == "new" and staged[PHASE] == STAGED
    with pytest.raises(RotationInProgress):
        stage_rotation(staged, "newer")


def test_a_staged_secret_is_committed_once_every_unit_accepts_it():
    staged = {PRIMARY: "old", EXTRA: "new", PHASE: STAGED}
    waiting = reports((digest("old"), {digest("new")}), (digest("old"), set()))
    assert advance_rotation(staged, waiting) is None
    ready = reports((digest("old"), {digest("new")}), (digest("old"), {digest("new")}))
    committed = advance_rotation(staged, ready)
    assert committed == {PRIMARY: "new", EXTRA: "old", PHASE: COMMITTED}


def test_the_old_secret_is_retired_once_every_unit_sends_the_new_one():
    committed = {PRIMARY: "new", EXTRA: "old", PHASE: COMMITTED, "jwt-secret": "j"}
    assert advance_rotation(committed, reports((digest("old"), {digest("new")}))) is None
    done = advance_rotation(committed, reports((digest("new"), {digest("old")})))
    assert done == {PRIMARY: "new", "jwt-secret": "j"}


def test_nothing_happens_without_a_rotation():
    assert advance_rotation({PRIMARY: "s"}, reports((digest("s"), set()))) is None


def test_unit_reports_round_trip_through_relation_data():
    fields = report_fields("p", ["a", "b"])
    report = report_from(fields)
    assert report.primary_digest == digest("p")
    assert report.accepted_digests == {digest("a"), digest("b")}


def status(voters, live):
    return {
        "quorium": {"servers": [{"address": a, "voter": True} for a in voters]},
        "members": [{"internal_url": a} for a in live],
    }


def test_restarting_a_voter_needs_a_healthy_majority_without_it():
    voters = ["http://a", "http://b", "http://c"]
    assert restart_is_safe(status(voters, voters), "http://a")
    assert not restart_is_safe(status(voters, ["http://a", "http://b"]), "http://a")


def test_learners_and_single_voters_can_always_restart():
    assert restart_is_safe(status(["http://a"], []), "http://a")
    assert restart_is_safe(status(["http://a", "http://b", "http://c"], []), "http://learner")
