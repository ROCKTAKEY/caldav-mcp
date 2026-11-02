import pytest

from caldav_mcp.calendars import Calendars
from caldav_mcp.dav import DavClient
from caldav_mcp.models import CalendarProperties, NewCalendar


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
