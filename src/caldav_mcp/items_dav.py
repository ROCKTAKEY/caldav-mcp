from datetime import UTC, date, datetime
from hashlib import sha256
from typing import Any
from urllib.parse import unquote, urlsplit
from xml.etree import ElementTree as ET
from zoneinfo import ZoneInfo

from icalendar import Calendar, Timezone
from pydantic import ValidationError

from .calendars import Calendars
from .dav import C, D, element, properties, tag, xml
from .errors import CalendarError
from .items import (
    EditedItem,
    RetrievedItem,
    create_document,
    describe_component,
    members,
    parse_document,
)
from .models import CalendarRef, NewItem, ResourceRef, Revision, Search


def dav_time(value: datetime) -> str:
    return value.astimezone(UTC).strftime("%Y%m%dT%H%M%SZ")


class Items:
    def __init__(self, calendars: Calendars, timezone: str = "UTC") -> None:
        self.calendars = calendars
        self.client = calendars.client
        self.timezone = timezone

    def check_ref(self, ref: ResourceRef) -> str:
        parent = self.calendars.check_ref(ref.calendar)
        url = self.client.resolve(ref.url)
        parent_path = unquote(urlsplit(parent).path)
        path = unquote(urlsplit(url).path)
        if (
            not path.startswith(parent_path)
            or not path[len(parent_path) :]
            or "/" in path[len(parent_path) :]
        ):
            raise CalendarError("invalid_target", "Resource must be a direct child of its calendar")
        return url

    async def get(self, ref: ResourceRef) -> RetrievedItem:
        url = self.check_ref(ref)
        response = await self.client.request("GET", url)
        document = parse_document(response.content)
        try:
            revision = Revision(etag=response.headers.get("ETag", ""))
        except ValidationError as exc:
            raise CalendarError(
                "invalid_response", "Calendar resource lacks a strong ETag"
            ) from exc
        return RetrievedItem(ref, revision, document)

    async def create(self, ref: CalendarRef, item: NewItem) -> RetrievedItem:
        document = create_document(item)
        info = await self.calendars.get(ref)
        info.require("write-content", item.kind)
        # Stable URLs and If-None-Match make a repeated creation fail instead of duplicating it.
        target = ResourceRef(
            calendar=ref, url=ref.url + sha256(item.uid.encode()).hexdigest() + ".ics"
        )
        return await self.write(target, document, {"If-None-Match": "*"})

    async def save(self, edited: EditedItem) -> RetrievedItem:
        self.check_ref(edited.ref)
        info = await self.calendars.get(edited.ref.calendar)
        info.require("write-content", members(edited.document)[0].name)
        return await self.write(edited.ref, edited.document, {"If-Match": edited.revision.etag})

    async def write(
        self, ref: ResourceRef, document: Calendar, conditions: dict[str, str]
    ) -> RetrievedItem:
        try:
            await self.client.request(
                "PUT",
                self.check_ref(ref),
                content=document.to_ical(),
                headers={"Content-Type": "text/calendar; charset=utf-8", **conditions},
            )
        except CalendarError as exc:
            exc.details["target"] = ref.model_dump()
            raise
        try:
            return await self.get(ref)
        except CalendarError as exc:
            raise CalendarError(
                "readback_failed",
                "Write succeeded; read the target before editing",
                write_succeeded=True,
                target=ref.model_dump(),
            ) from exc

    async def delete(self, ref: ResourceRef, revision: Revision) -> dict[str, str]:
        self.check_ref(ref)
        info = await self.calendars.get(ref.calendar)
        info.require("write-content")
        await self.client.request("DELETE", ref.url, headers={"If-Match": revision.etag})
        return {"status": "deleted"}

    async def search(self, ref: CalendarRef, query: Search) -> dict[str, Any]:
        info = await self.calendars.get(ref)
        info.require("read", query.kind)
        root = element(C, "calendar-query")
        prop = ET.SubElement(root, tag(D, "prop"))
        prop.append(element(D, "getetag"))
        data = ET.SubElement(prop, tag(C, "calendar-data"))
        if query.expand and query.period:
            data.append(
                element(
                    C, "expand", start=dav_time(query.period.start), end=dav_time(query.period.end)
                )
            )
        filters = ET.SubElement(root, tag(C, "filter"))
        vcalendar = ET.SubElement(filters, tag(C, "comp-filter"), {"name": "VCALENDAR"})
        component = ET.SubElement(vcalendar, tag(C, "comp-filter"), {"name": query.kind})
        if query.period:
            component.append(
                element(
                    C,
                    "time-range",
                    start=dav_time(query.period.start),
                    end=dav_time(query.period.end),
                )
            )
        if query.text:
            summary = ET.SubElement(component, tag(C, "prop-filter"), {"name": "SUMMARY"})
            summary.append(element(C, "text-match", query.text, collation="i;unicode-casemap"))
        if self.timezone != "UTC":
            timezone = Calendar()
            timezone.add("VERSION", "2.0")
            timezone.add("PRODID", "-//caldav-mcp//EN")
            first = query.period.start.date() if query.period else date(1970, 1, 1)
            last = query.period.end.date() if query.period else date(2100, 1, 1)
            timezone.add_component(
                Timezone.from_tzinfo(ZoneInfo(self.timezone), first_date=first, last_date=last)
            )
            root.append(element(C, "timezone", timezone.to_ical().decode()))
        response = await self.client.request(
            "REPORT",
            ref.url,
            content=xml(root),
            headers={"Depth": "1", "Content-Type": "application/xml; charset=utf-8"},
        )
        found, failures = [], []
        for row in sorted(properties(response.content), key=lambda row: row.href):
            if row.status >= 400 or row.failed:
                failures.append({"href": row.href, "status": row.status, "properties": row.failed})
                continue
            data_node = row.values.get(tag(C, "calendar-data"))
            etag_node = row.values.get(tag(D, "getetag"))
            if data_node is None or not data_node.text or etag_node is None or not etag_node.text:
                failures.append({"href": row.href, "code": "missing_calendar_data_or_etag"})
                continue
            target = ResourceRef(calendar=ref, url=self.client.resolve(row.href, ref.url))
            self.check_ref(target)
            try:
                revision = Revision(etag=etag_node.text)
                document = parse_document(data_node.text)
                for member in members(document):
                    if member.name == query.kind:
                        found.append(
                            {
                                "ref": target.model_dump(),
                                "revision": revision.model_dump(),
                                **describe_component(member),
                            }
                        )
            except (CalendarError, ValidationError) as exc:
                failures.append({"href": row.href, "code": "invalid_calendar", "message": str(exc)})
        return {
            "items": found[: query.limit],
            "truncated": len(found) > query.limit,
            "errors": failures,
            "complete": not failures and len(found) <= query.limit,
        }
