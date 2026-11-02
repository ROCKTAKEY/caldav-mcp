from datetime import UTC, datetime, timedelta
from typing import Any

from icalendar import Calendar

from caldav_mcp.calendars.dav import Calendars
from caldav_mcp.calendars.models import CalendarRef
from caldav_mcp.dav import C, element, xml
from caldav_mcp.errors import CalendarError
from caldav_mcp.items.dav import dav_time
from caldav_mcp.temporal import Period


def free_intervals(period: Period, busy: list[tuple[datetime, datetime]]) -> list[dict[str, str]]:
    cursor = period.start.astimezone(UTC)
    end = period.end.astimezone(UTC)
    free = []
    for start, stop in sorted(busy):
        start, stop = max(start, cursor), min(stop, end)
        if stop <= cursor or start >= end:
            continue
        if start > cursor:
            free.append({"start": cursor.isoformat(), "end": start.isoformat()})
        cursor = max(cursor, stop)
    if cursor < end:
        free.append({"start": cursor.isoformat(), "end": end.isoformat()})
    return free


class Availability:
    def __init__(self, calendars: Calendars) -> None:
        self.calendars = calendars

    async def query(self, ref: CalendarRef, period: Period) -> dict[str, Any]:
        url = self.calendars.check_ref(ref)
        root = element(C, "free-busy-query")
        root.append(
            element(C, "time-range", start=dav_time(period.start), end=dav_time(period.end))
        )
        # read-free-busy can be granted without permission to read calendar metadata or events.
        response = await self.calendars.client.request(
            "REPORT",
            url,
            content=xml(root),
            headers={"Depth": "1", "Content-Type": "application/xml; charset=utf-8"},
        )
        try:
            calendar = Calendar.from_ical(response.content)
            components = calendar.walk("VFREEBUSY")
            if not components:
                raise ValueError("Missing VFREEBUSY")
            busy = []
            intervals = []
            for component in components:
                values = component.get("FREEBUSY", [])
                for value in values if isinstance(values, list) else [values]:
                    if value.params.get("FBTYPE", "BUSY") == "FREE":
                        continue
                    start, end = value.dt
                    if isinstance(end, timedelta):
                        end = start + end
                    if (
                        not isinstance(start, datetime)
                        or not isinstance(end, datetime)
                        or not start.tzinfo
                        or not end.tzinfo
                    ):
                        raise ValueError("Invalid busy period")
                    start = max(start.astimezone(UTC), period.start.astimezone(UTC))
                    end = min(end.astimezone(UTC), period.end.astimezone(UTC))
                    if end <= start:
                        continue
                    busy.append(
                        {
                            "start": start.isoformat(),
                            "end": end.isoformat(),
                            "type": str(value.params.get("FBTYPE", "BUSY")),
                        }
                    )
                    intervals.append((start, end))
            return {
                "calendar": ref.model_dump(),
                "period": period.model_dump(mode="json"),
                "busy": busy,
                "free": free_intervals(period, intervals),
                "complete": True,
            }
        except (ValueError, TypeError, KeyError, AttributeError) as exc:
            raise CalendarError(
                "invalid_response", "Server returned invalid free/busy data"
            ) from exc
