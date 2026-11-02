from datetime import UTC, datetime

import pytest
from icalendar import Calendar

from caldav_mcp.errors import CalendarError
from caldav_mcp.items import create_document, edit_document, remove_occurrence
from caldav_mcp.models import ItemPatch, NewItem, OccurrenceScope, SeriesScope, UtcValue


def new_event():
    return NewItem(
        kind="VEVENT",
        uid="weekly",
        fields=ItemPatch(
            summary="Weekly",
            start={"kind": "zoned", "value": "2026-10-26T10:00:00", "timezone": "America/New_York"},
            end={"kind": "zoned", "value": "2026-10-26T11:00:00", "timezone": "America/New_York"},
            recurrence_rule="FREQ=WEEKLY;COUNT=5",
        ),
    )


def test_create_embeds_timezone_and_recurrence():
    doc = create_document(new_event())
    assert doc.walk("VEVENT")[0]["DTSTART"].dt.hour == 10
    assert len(doc.walk("VTIMEZONE")) == 1
    assert doc.walk("VEVENT")[0]["RRULE"]["COUNT"] == [5]


def test_task_rejects_due_and_duration_together():
    with pytest.raises(CalendarError):
        create_document(
            NewItem(
                kind="VTODO",
                uid="task",
                fields=ItemPatch(
                    start={"kind": "date", "value": "2026-11-02"},
                    due={"kind": "date", "value": "2026-11-03"},
                    duration="P1D",
                ),
            )
        )


def test_patch_preserves_vendor_properties_attendees_and_alarms():
    source = Calendar.from_ical("""BEGIN:VCALENDAR\r
VERSION:2.0\r
BEGIN:VTODO\r
UID:task\r
DTSTART:20261102T100000Z\r
DTSTAMP:20261102T000000Z\r
SUMMARY:Old\r
X-VENDOR;X-PARAM=hello:preserve me\r
ATTENDEE;PARTSTAT=ACCEPTED:mailto:alice@example.org\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
DESCRIPTION:Reminder\r
TRIGGER:-PT15M\r
END:VALARM\r
END:VTODO\r
END:VCALENDAR\r
""")
    updated = edit_document(source, ItemPatch(summary="New"), SeriesScope())
    task = updated.walk("VTODO")[0]
    assert str(task["SUMMARY"]) == "New"
    assert str(task["X-VENDOR"]) == "preserve me"
    assert task["X-VENDOR"].params["X-PARAM"] == "hello"
    assert task["ATTENDEE"].params["PARTSTAT"] == "ACCEPTED"
    assert len(task.subcomponents) == 1
    assert str(source.walk("VTODO")[0]["SUMMARY"]) == "Old"


def test_single_occurrence_edit_preserves_series_across_dst():
    doc = create_document(new_event())
    scope = OccurrenceScope(recurrence_id=UtcValue(value="2026-11-02T15:00:00Z"))
    result = edit_document(
        doc,
        ItemPatch(
            start={"kind": "utc", "value": "2026-11-02T16:00:00Z"},
            end={"kind": "utc", "value": "2026-11-02T17:00:00Z"},
        ),
        scope,
    )
    events = result.walk("VEVENT")
    assert len(events) == 2
    override = next(e for e in events if "RECURRENCE-ID" in e)
    assert override["RECURRENCE-ID"].dt == datetime(2026, 11, 2, 15, tzinfo=UTC)
    assert override["DTSTART"].dt.hour == 16
    assert "RRULE" not in override
    assert "RRULE" in events[0]


def test_removing_one_occurrence_adds_exdate_without_deleting_series():
    doc = create_document(new_event())
    scope = OccurrenceScope(recurrence_id=UtcValue(value="2026-11-02T15:00:00Z"))
    result = remove_occurrence(doc, scope)
    assert len(result.walk("VEVENT")) == 1
    assert result.walk("VEVENT")[0]["EXDATE"].dts[0].dt == scope.recurrence_id.native()


def test_nonexistent_occurrence_cannot_be_edited():
    doc = create_document(new_event())
    with pytest.raises(CalendarError):
        edit_document(
            doc,
            ItemPatch(summary="wrong day"),
            OccurrenceScope(recurrence_id=UtcValue(value="2026-11-03T15:00:00Z")),
        )


def test_patch_omission_and_explicit_removal_are_distinct():
    doc = create_document(new_event())
    doc.walk("VEVENT")[0].add("description", "Details")
    assert (
        "DESCRIPTION"
        in edit_document(doc, ItemPatch(summary="Title"), SeriesScope()).walk("VEVENT")[0]
    )
    assert (
        "DESCRIPTION"
        not in edit_document(doc, ItemPatch(description=None), SeriesScope()).walk("VEVENT")[0]
    )


def test_radicale_unsupported_future_scope_is_rejected_at_boundary():
    from pydantic import TypeAdapter, ValidationError

    from caldav_mcp.models import Scope

    with pytest.raises(ValidationError):
        TypeAdapter(Scope).validate_python(
            {"kind": "future", "recurrence_id": {"kind": "utc", "value": "2026-11-02T15:00:00Z"}}
        )


def test_absolute_alarm_does_not_require_task_start():
    item = NewItem(
        kind="VTODO",
        uid="alarm",
        fields=ItemPatch.model_validate(
            {
                "summary": "Reminder",
                "alarms": [
                    {
                        "description": "Reminder",
                        "trigger": {"kind": "utc", "value": "2026-11-02T10:00:00Z"},
                    }
                ],
            }
        ),
    )
    alarm = create_document(item).walk("VALARM")[0]
    assert alarm["TRIGGER"].dt == datetime(2026, 11, 2, 10, tzinfo=UTC)
    assert alarm["TRIGGER"].params["VALUE"] == "DATE-TIME"


def test_invalid_recurrence_interval_is_rejected():
    with pytest.raises(CalendarError):
        create_document(
            new_event().model_copy(
                update={
                    "fields": new_event().fields.model_copy(
                        update={"recurrence_rule": "FREQ=WEEKLY;INTERVAL=0"}
                    )
                }
            )
        )
