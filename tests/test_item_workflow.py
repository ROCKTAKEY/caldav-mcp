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


@pytest.mark.asyncio
async def test_recurrence_and_advanced_fields_roundtrip_radicale(settings):
    from caldav_mcp.models import OccurrenceScope, Period, UtcValue

    client = DavClient(settings.accounts[0])
    calendars, items = Calendars(client), Items(Calendars(client))
    try:
        calendar = await calendars.create(NewCalendar(account="alice", name="Recurrences"))
        created = await items.create(
            calendar.ref,
            NewItem(
                kind="VEVENT",
                uid=str(uuid4()),
                fields=ItemPatch(
                    summary="Weekly",
                    start={
                        "kind": "zoned",
                        "value": "2026-10-26T10:00:00",
                        "timezone": "America/New_York",
                    },
                    end={
                        "kind": "zoned",
                        "value": "2026-10-26T11:00:00",
                        "timezone": "America/New_York",
                    },
                    recurrence_rule="FREQ=WEEKLY;COUNT=5",
                    categories=["Work", "Planning"],
                    classification="PRIVATE",
                    attachments=[
                        {"uri": "https://example.org/agenda.pdf", "media_type": "application/pdf"}
                    ],
                    alarms=[{"description": "Meeting", "trigger": "-PT15M"}],
                ),
            ),
        )
        updated = await items.save(
            created.edit(
                ItemPatch(
                    start={
                        "kind": "zoned",
                        "value": "2026-11-02T11:00:00",
                        "timezone": "America/New_York",
                    },
                    end={
                        "kind": "zoned",
                        "value": "2026-11-02T12:00:00",
                        "timezone": "America/New_York",
                    },
                ),
                OccurrenceScope(recurrence_id=UtcValue(value="2026-11-02T15:00:00Z")),
            )
        )
        event = updated.document.walk("VEVENT")[0]
        assert str(event["CLASS"]) == "PRIVATE"
        assert str(event["ATTACH"]) == "https://example.org/agenda.pdf"
        assert event.subcomponents[0].name == "VALARM"
        query = Search(
            kind="VEVENT",
            calendars=[calendar.ref],
            expand=True,
            period=Period(start="2026-10-26T00:00:00Z", end="2026-11-24T00:00:00Z"),
        )
        expanded = await items.search(calendar.ref, query)
        assert expanded["complete"], expanded
        assert len(expanded["items"]) == 5
        from datetime import datetime

        starts = sorted(datetime.fromisoformat(i["start"]["value"]) for i in expanded["items"])
        assert [t.hour for t in starts] == [14, 16, 15, 15, 15]
        await calendars.delete(calendar.ref)
    finally:
        await client.close()
