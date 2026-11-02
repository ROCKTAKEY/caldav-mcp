from uuid import uuid4

import pytest

from caldav_mcp.availability import Availability
from caldav_mcp.calendars import Calendars
from caldav_mcp.dav import DavClient
from caldav_mcp.items_dav import Items
from caldav_mcp.models import ItemPatch, NewCalendar, NewItem, Period


@pytest.mark.asyncio
async def test_free_busy_uses_server_recurrences_and_transparency(settings):
    client = DavClient(settings.accounts[0])
    calendars = Calendars(client)
    items = Items(calendars)
    try:
        calendar = await calendars.create(NewCalendar(account="alice", name="Availability"))
        await items.create(
            calendar.ref,
            NewItem(
                kind="VEVENT",
                uid=str(uuid4()),
                fields=ItemPatch(
                    summary="Private meeting",
                    start={"kind": "utc", "value": "2026-11-02T10:00:00Z"},
                    end={"kind": "utc", "value": "2026-11-02T11:00:00Z"},
                    recurrence_rule="FREQ=DAILY;COUNT=3",
                ),
            ),
        )
        await items.create(
            calendar.ref,
            NewItem(
                kind="VEVENT",
                uid=str(uuid4()),
                fields=ItemPatch(
                    summary="Reminder",
                    start={"kind": "utc", "value": "2026-11-03T12:00:00Z"},
                    end={"kind": "utc", "value": "2026-11-03T13:00:00Z"},
                    transparency="TRANSPARENT",
                ),
            ),
        )
        result = await Availability(calendars).query(
            calendar.ref, Period(start="2026-11-03T09:00:00Z", end="2026-11-03T14:00:00Z")
        )
        assert len(result["busy"]) == 1
        assert result["busy"][0]["start"] == "2026-11-03T10:00:00+00:00"
        assert len(result["free"]) == 2
        assert "Private meeting" not in str(result)
        await calendars.delete(calendar.ref)
    finally:
        await client.close()
