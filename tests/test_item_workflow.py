from uuid import uuid4

import pytest

from caldav_mcp.calendars import Calendars
from caldav_mcp.dav import DavClient
from caldav_mcp.errors import CalendarError
from caldav_mcp.items_dav import Items
from caldav_mcp.models import ItemPatch, NewCalendar, NewItem, Search, SeriesScope


@pytest.mark.asyncio
async def test_task_crud_conflicts_and_complete_reopen(settings):
    client = DavClient(settings.accounts[0])
    calendars = Calendars(client)
    items = Items(calendars)
    try:
        calendar = await calendars.create(NewCalendar(account="alice", name="Tasks"))
        spec = NewItem(
            kind="VTODO",
            uid=str(uuid4()),
            fields=ItemPatch(summary="Review", due={"kind": "date", "value": "2026-11-09"}),
        )
        created = await items.create(calendar.ref, spec)
        assert created.document.walk("VTODO")[0]["SUMMARY"] == "Review"
        with pytest.raises(CalendarError) as duplicate:
            await items.create(calendar.ref, spec)
        assert duplicate.value.code == "conflict"
        result = await items.search(
            calendar.ref, Search(kind="VTODO", calendars=[calendar.ref], text="Review")
        )
        assert len(result["items"]) == 1
        completed = await items.save(created.edit(ItemPatch(status="COMPLETED"), SeriesScope()))
        assert int(completed.document.walk("VTODO")[0]["PERCENT-COMPLETE"]) == 100
        with pytest.raises(CalendarError) as stale:
            await items.save(created.edit(ItemPatch(summary="stale"), SeriesScope()))
        assert stale.value.code == "conflict"
        reopened = await items.save(completed.edit(ItemPatch(status="NEEDS-ACTION"), SeriesScope()))
        assert "COMPLETED" not in reopened.document.walk("VTODO")[0]
        await items.delete(reopened.ref, reopened.revision)
        with pytest.raises(CalendarError) as deleted:
            await items.get(reopened.ref)
        assert deleted.value.code == "not_found"
        await calendars.delete(calendar.ref)
    finally:
        await client.close()
