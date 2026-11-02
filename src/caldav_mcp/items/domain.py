from copy import deepcopy
from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from typing import Any

import recurring_ical_events
from dateutil.rrule import rrulestr
from icalendar import Alarm, Calendar, Component, Event, Journal, Todo, vRecur

from caldav_mcp.errors import CalendarError
from caldav_mcp.items.models import (
    ItemPatch,
    NewItem,
    OccurrenceScope,
    ResourceRef,
    Revision,
    Scope,
)


@dataclass(frozen=True)
class RetrievedItem:
    ref: ResourceRef
    revision: Revision
    document: Calendar

    def edit(self, patch: ItemPatch, scope: Scope) -> "EditedItem":
        return EditedItem(self.ref, self.revision, edit_document(self.document, patch, scope))


@dataclass(frozen=True)
class EditedItem:
    ref: ResourceRef
    revision: Revision
    document: Calendar


SIMPLE_PROPERTIES = {
    "summary": "SUMMARY",
    "description": "DESCRIPTION",
    "location": "LOCATION",
    "start": "DTSTART",
    "end": "DTEND",
    "due": "DUE",
    "duration": "DURATION",
    "status": "STATUS",
    "percent_complete": "PERCENT-COMPLETE",
    "priority": "PRIORITY",
    "categories": "CATEGORIES",
    "classification": "CLASS",
    "transparency": "TRANSP",
    "recurrence_rule": "RRULE",
    "recurrence_dates": "RDATE",
    "exception_dates": "EXDATE",
    "related_to": "RELATED-TO",
    "attachments": "ATTACH",
}


def members(calendar: Calendar) -> list[Component]:
    return [c for c in calendar.subcomponents if c.name in ("VEVENT", "VTODO", "VJOURNAL")]


def parse_document(data: str | bytes) -> Calendar:
    try:
        calendar = Calendar.from_ical(data)
        items = members(calendar)
        if not items or len({(c.name, str(c.get("UID", ""))) for c in items}) != 1:
            raise ValueError("Expected a single UID and component type")
        if not items[0].get("UID") or any(c.errors for c in calendar.walk()):
            raise ValueError("Missing UID or malformed properties")
        return calendar
    except (ValueError, KeyError, TypeError) as exc:
        raise CalendarError("invalid_calendar", "Cannot parse calendar resource") from exc


def native(component: Component, name: str) -> Any:
    value = component.get(name)
    return value.dt if value is not None and hasattr(value, "dt") else None


def same_time_type(a: date | datetime, b: date | datetime) -> bool:
    if isinstance(a, datetime) and isinstance(b, datetime):
        return (a.tzinfo is None) == (b.tzinfo is None)
    return type(a) is type(b)


def validate_component(component: Component) -> None:
    start, end, due = (native(component, p) for p in ("DTSTART", "DTEND", "DUE"))
    duration = native(component, "DURATION")
    kind = component.name
    assert kind is not None
    if kind == "VEVENT" and start is None:
        raise CalendarError("invalid_input", "An event requires start")
    if kind != "VEVENT" and end is not None:
        raise CalendarError("invalid_input", "end is only valid for events")
    if kind != "VTODO" and any(p in component for p in ("DUE", "PERCENT-COMPLETE")):
        raise CalendarError("invalid_input", "due and percent_complete are only valid for tasks")
    if duration is not None:
        if kind == "VJOURNAL" or start is None or end is not None or due is not None:
            raise CalendarError("invalid_input", "duration requires start and excludes end/due")
        if duration <= timedelta(0):
            raise CalendarError("invalid_input", "duration must be positive")
        if not isinstance(start, datetime) and duration.seconds:
            raise CalendarError("invalid_input", "Date-only duration must be whole days")
    for finish in (end, due):
        if finish is not None and start is not None:
            if not same_time_type(start, finish) or finish <= start:
                raise CalendarError("invalid_input", "end/due must match start type and be later")
    statuses = {
        "VEVENT": {"TENTATIVE", "CONFIRMED", "CANCELLED"},
        "VTODO": {"NEEDS-ACTION", "IN-PROCESS", "COMPLETED", "CANCELLED"},
        "VJOURNAL": {"DRAFT", "FINAL", "CANCELLED"},
    }
    status = component.get("STATUS")
    if status is not None and str(status) not in statuses[kind]:
        raise CalendarError("invalid_input", f"Invalid status for {kind}")
    if "RRULE" in component:
        if start is None:
            raise CalendarError("invalid_input", "Recurrence requires start")
        rule = component["RRULE"]
        if not isinstance(rule, vRecur):
            raise CalendarError("invalid_input", "Expected one recurrence rule")
        for positive in ("COUNT", "INTERVAL"):
            if positive in rule and (len(rule[positive]) != 1 or int(rule[positive][0]) <= 0):
                raise CalendarError("invalid_input", f"RRULE {positive} must be a positive integer")
        if "COUNT" in rule and "UNTIL" in rule:
            raise CalendarError("invalid_input", "RRULE cannot contain COUNT and UNTIL together")
        try:
            dtstart = (
                start
                if isinstance(start, datetime)
                else datetime.combine(start, datetime.min.time())
            )
            rrulestr(rule.to_ical().decode(), dtstart=dtstart)
        except (ValueError, TypeError, OverflowError) as exc:
            raise CalendarError("invalid_input", "Invalid recurrence rule for start") from exc
    for name in ("RDATE", "EXDATE"):
        values = component.get(name, [])
        for prop in values if isinstance(values, list) else [values]:
            for value in prop.dts:
                if start is None or not same_time_type(start, value.dt):
                    raise CalendarError("invalid_input", f"{name} must match start type")
    for alarm in component.subcomponents:
        if alarm.name == "VALARM":
            trigger = alarm.get("TRIGGER")
            if trigger is not None and isinstance(trigger.dt, timedelta):
                if trigger.params.get("RELATED", "START") == "END":
                    if end is None and due is None and duration is None:
                        raise CalendarError(
                            "invalid_input", "End-relative alarm needs end/due/duration"
                        )
                elif start is None:
                    raise CalendarError("invalid_input", "Start-relative alarm needs start")


def apply_patch(component: Component, patch: ItemPatch) -> None:
    for field in patch.model_fields_set:
        value = getattr(patch, field)
        if field == "alarms":
            component.subcomponents = [c for c in component.subcomponents if c.name != "VALARM"]
            for spec in value or []:
                alarm = Alarm()
                alarm.add("ACTION", "DISPLAY")
                alarm.add("DESCRIPTION", spec.description)
                if isinstance(spec.trigger, timedelta):
                    alarm.add("TRIGGER", spec.trigger, parameters={"RELATED": spec.relative_to})
                else:
                    alarm.add("TRIGGER", spec.trigger.native(), parameters={"VALUE": "DATE-TIME"})
                component.add_component(alarm)
            continue
        prop = SIMPLE_PROPERTIES[field]
        component.pop(prop, None)
        if value is None:
            continue
        if field in ("start", "end", "due"):
            component.add(prop, value.native())
        elif field in ("recurrence_dates", "exception_dates"):
            for time in value:
                component.add(prop, time.native())
        elif field == "recurrence_rule":
            try:
                component.add(prop, vRecur.from_ical(value))
            except (ValueError, TypeError) as exc:
                raise CalendarError("invalid_input", "Malformed recurrence rule") from exc
        elif field == "related_to":
            for relation in value:
                component.add(prop, relation.uid, parameters={"RELTYPE": relation.relationship})
        elif field == "attachments":
            for attachment in value:
                parameters = {"VALUE": "URI"}
                if attachment.media_type:
                    parameters["FMTTYPE"] = attachment.media_type
                component.add(prop, attachment.uri, parameters=parameters)
        else:
            component.add(prop, value)
    if component.name == "VTODO" and "status" in patch.model_fields_set:
        if patch.status == "COMPLETED":
            component["PERCENT-COMPLETE"] = 100
            component.pop("COMPLETED", None)
            component.add("COMPLETED", datetime.now(UTC))
        else:
            component.pop("COMPLETED", None)
            if patch.status == "NEEDS-ACTION":
                component["PERCENT-COMPLETE"] = 0
    component.pop("DTSTAMP", None)
    component.add("DTSTAMP", datetime.now(UTC))
    component.pop("LAST-MODIFIED", None)
    component.add("LAST-MODIFIED", datetime.now(UTC))
    component["SEQUENCE"] = int(component.get("SEQUENCE", 0)) + 1
    validate_component(component)


def add_timezones(calendar: Calendar) -> None:
    # RFC 4791 requires referenced TZIDs to be defined in the stored resource.
    # VTIMEZONE preserves server/client interoperability without rewriting existing definitions.
    years = [
        value.dt.year
        for c in members(calendar)
        for key in ("DTSTART", "DTEND", "DUE")
        if (value := c.get(key)) is not None and hasattr(value, "dt")
    ]
    first = min(years, default=datetime.now(UTC).year)
    last = max(years, default=first)
    calendar.add_missing_timezones(
        first_date=date(max(1, first - 1), 1, 1),
        last_date=date(min(9999, last + 100), 12, 31),
    )


def create_document(item: NewItem) -> Calendar:
    calendar = Calendar()
    calendar.add("VERSION", "2.0")
    calendar.add("PRODID", "-//caldav-mcp//EN")
    component = {"VEVENT": Event, "VTODO": Todo, "VJOURNAL": Journal}[item.kind]()
    component.add("UID", item.uid)
    component.add("CREATED", datetime.now(UTC))
    apply_patch(component, item.fields)
    calendar.add_component(component)
    add_timezones(calendar)
    return calendar


def select_occurrence(calendar: Calendar, scope: OccurrenceScope) -> Component:
    items = members(calendar)
    target = scope.recurrence_id.native()
    master = next((c for c in items if "RECURRENCE-ID" not in c), None)
    for c in items:
        if native(c, "RECURRENCE-ID") == target:
            return c
    if master is None or not any(p in master for p in ("RRULE", "RDATE")):
        raise CalendarError("not_found", "No recurring master or matching exception")
    start = native(master, "DTSTART")
    if not same_time_type(start, target):
        raise CalendarError("invalid_input", "recurrence_id must match start's date/time type")
    if isinstance(start, datetime) and start.tzinfo is not None and isinstance(target, datetime):
        target = target.astimezone(start.tzinfo)
    try:
        matches = recurring_ical_events.of(calendar, components=[master.name]).at(target)
    except (ValueError, TypeError, KeyError) as exc:
        raise CalendarError("invalid_calendar", "Cannot expand this recurrence") from exc
    for occurrence in matches:
        if native(occurrence, "RECURRENCE-ID") == target:
            occurrence = deepcopy(occurrence)
            for prop in ("RRULE", "RDATE", "EXDATE"):
                occurrence.pop(prop, None)
            calendar.add_component(occurrence)
            return occurrence
    raise CalendarError("not_found", "recurrence_id is not an occurrence of this series")


def edit_document(calendar: Calendar, patch: ItemPatch, scope: Scope) -> Calendar:
    result = deepcopy(calendar)
    if isinstance(scope, OccurrenceScope):
        if patch.model_fields_set & {"recurrence_rule", "recurrence_dates", "exception_dates"}:
            raise CalendarError("invalid_input", "Edit recurrence rules with series scope")
        target = select_occurrence(result, scope)
        if target["RECURRENCE-ID"].params.get("RANGE"):
            raise CalendarError(
                "unsupported", "Cannot edit a THISANDFUTURE anchor as one occurrence"
            )
        targets = [target]
    else:
        targets = [c for c in members(result) if "RECURRENCE-ID" not in c]
        if len(targets) != 1:
            raise CalendarError("invalid_calendar", "Series edit requires exactly one master")
    for component in targets:
        apply_patch(component, patch)
    add_timezones(result)
    return result


def remove_occurrence(calendar: Calendar, scope: OccurrenceScope) -> Calendar:
    result = deepcopy(calendar)
    selected = select_occurrence(result, scope)
    target = native(selected, "RECURRENCE-ID")
    result.subcomponents.remove(selected)
    master = next((c for c in members(result) if "RECURRENCE-ID" not in c), None)
    if master is None:
        raise CalendarError("invalid_calendar", "Cannot exclude occurrence without its master")
    master.add("EXDATE", target)
    apply_patch(master, ItemPatch())
    return result


def json_time(value: Any) -> dict[str, Any] | None:
    if value is None or not hasattr(value, "dt"):
        return None
    dt = value.dt
    if isinstance(dt, datetime):
        tzid = value.params.get("TZID")
        if tzid:
            return {
                "kind": "zoned",
                "value": dt.replace(tzinfo=None).isoformat(),
                "timezone": str(tzid),
            }
        if dt.tzinfo is not None:
            return {"kind": "utc", "value": dt.astimezone(UTC).isoformat()}
        return {"kind": "floating", "value": dt.isoformat()}
    if isinstance(dt, date):
        return {"kind": "date", "value": dt.isoformat()}
    return None


def describe_component(component: Component, detailed: bool = False) -> dict[str, Any]:
    result: dict[str, Any] = {"kind": component.name, "uid": str(component.get("UID", ""))}
    for key in ("SUMMARY", "STATUS", "LOCATION", "PERCENT-COMPLETE", "PRIORITY"):
        if key in component:
            result[key.lower().replace("-", "_")] = str(component[key])
    for prop, key in (
        ("DTSTART", "start"),
        ("DTEND", "end"),
        ("DUE", "due"),
        ("RECURRENCE-ID", "recurrence_id"),
    ):
        if prop in component:
            result[key] = json_time(component[prop])
    if "RRULE" in component:
        result["recurrence_rule"] = ical_text(component["RRULE"])
    if detailed:
        result["icalendar"] = component.to_ical().decode()
    return result


def ical_text(value: Any) -> str:
    serialized = value.to_ical()
    return serialized.decode() if isinstance(serialized, bytes) else str(serialized)
