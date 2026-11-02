from datetime import date
from typing import Any
from uuid import uuid4
from xml.etree import ElementTree as ET
from zoneinfo import ZoneInfo

from icalendar import Calendar, Timezone

from caldav_mcp.calendars.models import CalendarProperties, CalendarRef, NewCalendar
from caldav_mcp.contracts import Model
from caldav_mcp.dav import A, C, D, DavClient, Properties, element, properties, tag, xml
from caldav_mcp.errors import CalendarError

CALENDAR_PROPERTIES = [
    tag(C, "calendar-timezone"),
    tag(D, "resourcetype"),
    tag(D, "displayname"),
    tag(C, "calendar-description"),
    tag(A, "calendar-color"),
    tag(C, "supported-calendar-component-set"),
    tag(D, "current-user-privilege-set"),
    tag(D, "supported-report-set"),
    tag(D, "owner"),
]


class CalendarInfo(Model):
    ref: CalendarRef
    name: str | None
    description: str | None
    color: str | None
    timezone_icalendar: str | None
    components: list[str] | None
    privileges: list[str] | None
    reports: list[str] | None
    owner: str | None
    unavailable_properties: dict[str, int]

    def require(self, privilege: str, component: str | None = None) -> None:
        if component and self.components is not None and component not in self.components:
            raise CalendarError("unsupported", f"Calendar does not support {component}")
        if self.privileges is not None:
            alternatives = {tag(D, "all"), tag(D, privilege)}
            if privilege in ("write-content", "write-properties", "bind", "unbind"):
                alternatives.add(tag(D, "write"))
            if not alternatives.intersection(self.privileges):
                raise CalendarError("forbidden", f"Calendar does not grant {privilege}")


class Calendars:
    def __init__(self, client: DavClient) -> None:
        self.client = client

    def ref(self, url: str) -> CalendarRef:
        return CalendarRef(account=self.client.config.id, url=self.client.resolve(url))

    def check_ref(self, ref: CalendarRef) -> str:
        if ref.account != self.client.config.id:
            raise CalendarError("invalid_target", "Calendar belongs to a different account")
        url = self.client.resolve(ref.url)
        if not url.endswith("/"):
            raise CalendarError("invalid_target", "Calendar URL must end with /")
        return url

    def info(self, row: Properties, base: str) -> CalendarInfo | None:
        resource_type = row.values.get(tag(D, "resourcetype"))
        if resource_type is None or resource_type.find(tag(C, "calendar")) is None:
            return None

        def text(name: str) -> str | None:
            node = row.values.get(name)
            return node.text if node is not None else None

        def descendants(name: str, path: str) -> list[Any] | None:
            node = row.values.get(name)
            return list(node.findall(path)) if node is not None else None

        components = descendants(tag(C, "supported-calendar-component-set"), tag(C, "comp"))
        privileges = descendants(tag(D, "current-user-privilege-set"), f"{tag(D, 'privilege')}/*")
        reports = descendants(tag(D, "supported-report-set"), f".//{tag(D, 'report')}/*")
        owner = row.values.get(tag(D, "owner"))
        return CalendarInfo(
            ref=self.ref(self.client.resolve(row.href, base)),
            name=text(tag(D, "displayname")),
            timezone_icalendar=text(tag(C, "calendar-timezone")),
            description=text(tag(C, "calendar-description")),
            color=text(tag(A, "calendar-color")),
            components=[n.attrib["name"] for n in components] if components is not None else None,
            privileges=[n.tag for n in privileges] if privileges is not None else None,
            reports=[n.tag for n in reports] if reports is not None else None,
            owner=owner.findtext(tag(D, "href")) if owner is not None else None,
            unavailable_properties=row.failed,
        )

    async def get(self, ref: CalendarRef) -> CalendarInfo:
        url = self.check_ref(ref)
        rows = await self.client.propfind(url, CALENDAR_PROPERTIES)
        for row in rows:
            if self.client.resolve(row.href, url) == url:
                info = self.info(row, url)
                if info:
                    return info
        raise CalendarError("not_calendar", "Target is not an accessible calendar collection")

    async def home_sets(self) -> list[str]:
        base = self.client.config.url
        rows = await self.client.propfind(
            base, [tag(D, "current-user-principal"), tag(C, "calendar-home-set")]
        )
        homes = self.hrefs(rows, tag(C, "calendar-home-set"), base)
        if homes:
            return homes
        principals = self.hrefs(rows, tag(D, "current-user-principal"), base)
        if not principals:
            raise CalendarError(
                "discovery_failed", "Server did not return a current-user-principal"
            )
        rows = await self.client.propfind(principals[0], [tag(C, "calendar-home-set")])
        homes = self.hrefs(rows, tag(C, "calendar-home-set"), principals[0])
        if not homes:
            raise CalendarError("discovery_failed", "Server did not return a calendar-home-set")
        return homes

    def hrefs(self, rows: list[Properties], name: str, base: str) -> list[str]:
        result = []
        for row in rows:
            prop = row.values.get(name)
            if prop is not None:
                for href in prop.findall(tag(D, "href")):
                    if href.text:
                        result.append(self.client.resolve(href.text, base))
        return result

    async def list(self) -> list[CalendarInfo]:
        result = {}
        for home in await self.home_sets():
            for row in await self.client.propfind(home, CALENDAR_PROPERTIES, depth=1):
                if row.status >= 400:
                    raise CalendarError(
                        "partial_response", "A calendar could not be listed", href=row.href
                    )
                info = self.info(row, home)
                if info:
                    result[info.ref.url] = info
        for extra in self.client.config.calendars:
            info = await self.get(self.ref(extra))
            result[info.ref.url] = info
        return list(result.values())

    async def create(self, spec: NewCalendar) -> CalendarInfo:
        if spec.account != self.client.config.id:
            raise CalendarError("invalid_target", "Account mismatch")
        home = (await self.home_sets())[0]
        ref = self.ref(home.rstrip("/") + "/" + str(uuid4()) + "/")
        root = element(C, "mkcalendar")
        prop = ET.SubElement(ET.SubElement(root, tag(D, "set")), tag(D, "prop"))
        prop.append(element(D, "displayname", spec.name))
        if spec.description is not None:
            prop.append(element(C, "calendar-description", spec.description))
        if spec.timezone is not None:
            prop.append(element(C, "calendar-timezone", timezone_document(spec.timezone)))
        supported = ET.SubElement(prop, tag(C, "supported-calendar-component-set"))
        for kind in spec.components:
            supported.append(element(C, "comp", name=kind))
        try:
            await self.client.request(
                "MKCALENDAR",
                ref.url,
                content=xml(root),
                headers={"Content-Type": "application/xml; charset=utf-8"},
            )
        except CalendarError as exc:
            exc.details["target"] = ref.model_dump()
            raise
        try:
            return await self.get(ref)
        except CalendarError as exc:
            raise CalendarError(
                "readback_failed",
                "Calendar created; metadata retrieval failed",
                target=ref.model_dump(),
                write_succeeded=True,
            ) from exc

    async def update(self, ref: CalendarRef, changes: CalendarProperties) -> CalendarInfo:
        current = await self.get(ref)
        current.require("write-properties")
        root = element(D, "propertyupdate")
        mapping = {
            "name": (D, "displayname"),
            "description": (C, "calendar-description"),
            "color": (A, "calendar-color"),
            "timezone": (C, "calendar-timezone"),
        }
        if not changes.model_fields_set:
            return current
        for field in sorted(changes.model_fields_set):
            value = getattr(changes, field)
            if field == "timezone" and value is not None:
                value = timezone_document(value)
            action = ET.SubElement(root, tag(D, "remove" if value is None else "set"))
            prop = ET.SubElement(action, tag(D, "prop"))
            prop.append(element(*mapping[field], value))
        response = await self.client.request(
            "PROPPATCH",
            ref.url,
            content=xml(root),
            headers={"Content-Type": "application/xml; charset=utf-8"},
        )
        if response.status_code == 207:
            failed = {
                name: status
                for row in properties(response.content)
                for name, status in row.failed.items()
            }
            if failed:
                raise CalendarError(
                    "property_update_failed",
                    "Calendar properties were not all updated",
                    properties=failed,
                )
        return await self.get(ref)

    async def delete(self, ref: CalendarRef) -> dict[str, str]:
        await self.get(ref)
        await self.client.request("DELETE", ref.url)
        return {"status": "deleted"}


def timezone_document(name: str) -> str:
    calendar = Calendar()
    calendar.add("VERSION", "2.0")
    calendar.add("PRODID", "-//caldav-mcp//EN")
    calendar.add_component(
        Timezone.from_tzinfo(
            ZoneInfo(name), first_date=date(1900, 1, 1), last_date=date(2100, 12, 31)
        )
    )
    return calendar.to_ical().decode()
