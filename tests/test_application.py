from uuid import uuid4

import pytest

from caldav_mcp.application import Application
from caldav_mcp.calendars.models import CalendarRef, NewCalendar
from caldav_mcp.items.models import ItemPatch, NewItem, Search
from caldav_mcp.temporal import Period


@pytest.mark.asyncio
async def test_federated_query_marks_partial_failure_and_never_claims_free_time(settings):
    app = Application(settings)
    try:
        first = await app.calendars("alice").create(NewCalendar(account="alice", name="Personal"))
        second = await app.calendars("bob").create(NewCalendar(account="bob", name="Work"))
        uid = str(uuid4())
        for ref in (first.ref, second.ref):
            await app.items(ref.account).create(
                ref, NewItem(kind="VTODO", uid=uid, fields=ItemPatch(summary=ref.account))
            )
        query = Search(kind="VTODO", calendars=[first.ref, second.ref])
        result = await app.search(query)
        assert result["complete"]
        assert {i["ref"]["calendar"]["account"] for i in result["items"]} == {"alice", "bob"}
        missing = CalendarRef(account="bob", url=settings.accounts[1].url + "bob/missing/")
        partial = await app.search(query.model_copy(update={"calendars": [first.ref, missing]}))
        assert not partial["complete"]
        assert len(partial["items"]) == 1
        assert len(partial["errors"]) == 1
        availability = await app.free_busy(
            [first.ref, missing], Period(start="2026-11-02T00:00:00Z", end="2026-11-03T00:00:00Z")
        )
        assert not availability["complete"]
        assert availability["free"] is None
    finally:
        await app.close()
