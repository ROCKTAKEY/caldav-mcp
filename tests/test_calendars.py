import pytest

from caldav_mcp.calendars.dav import Calendars
from caldav_mcp.calendars.models import CalendarProperties, NewCalendar
from caldav_mcp.dav import DavClient


@pytest.mark.asyncio
async def test_calendar_lifecycle_and_discovery(settings):
    client = DavClient(settings.accounts[0])
    calendars = Calendars(client)
    try:
        created = await calendars.create(NewCalendar(account="alice", name="Planning"))
        assert created.name == "Planning"
        found = await calendars.list()
        assert created.ref in [c.ref for c in found]
        assert "VTODO" in created.components
        assert "{DAV:}write" in created.privileges or "{DAV:}write-content" in created.privileges
        updated = await calendars.update(
            created.ref, CalendarProperties(name="Renamed", description="Team", color="#123456")
        )
        assert updated.name == "Renamed"
        assert updated.description == "Team"
        await calendars.delete(created.ref)
        assert created.ref not in [c.ref for c in await calendars.list()]
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_calendar_timezone_roundtrips(settings):
    client = DavClient(settings.accounts[0])
    calendars = Calendars(client)
    try:
        created = await calendars.create(
            NewCalendar.model_validate(
                {"account": "alice", "name": "Tokyo", "timezone": "Asia/Tokyo"}
            )
        )
        assert "TZID:Asia/Tokyo" in created.timezone_icalendar
        changed = await calendars.update(
            created.ref, CalendarProperties.model_validate({"timezone": "Europe/Berlin"})
        )
        assert "TZID:Europe/Berlin" in changed.timezone_icalendar
        await calendars.delete(created.ref)
    finally:
        await client.close()
