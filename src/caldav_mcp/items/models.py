from datetime import timedelta
from typing import Annotated, Literal, Self
from urllib.parse import urlsplit

from pydantic import (
    Field,
    field_validator,
    model_validator,
)

from caldav_mcp.calendars.models import CalendarRef, ComponentKind
from caldav_mcp.contracts import Model
from caldav_mcp.temporal import Period, TimeValue, UtcValue


class ResourceRef(Model):
    calendar: CalendarRef
    url: str


class Revision(Model):
    etag: str = Field(pattern=r'^"[^"\r\n]*"$')


class AlarmSpec(Model):
    description: str
    trigger: timedelta | UtcValue
    relative_to: Literal["START", "END"] = "START"


class Relation(Model):
    uid: str = Field(min_length=1)
    relationship: Literal["PARENT", "CHILD", "SIBLING"] = "PARENT"


class Attachment(Model):
    uri: str
    media_type: str | None = None

    @field_validator("uri")
    @classmethod
    def absolute_uri(cls, value: str) -> str:
        if not urlsplit(value).scheme or any(c in value for c in "\r\n"):
            raise ValueError("Attachment requires an absolute URI")
        return value


class ItemPatch(Model):
    """Omitted fields are preserved; null removes a property."""

    summary: str | None = None
    description: str | None = None
    location: str | None = None
    start: TimeValue | None = None
    end: TimeValue | None = None
    due: TimeValue | None = None
    duration: timedelta | None = None
    status: (
        Literal[
            "TENTATIVE",
            "CONFIRMED",
            "CANCELLED",
            "NEEDS-ACTION",
            "IN-PROCESS",
            "COMPLETED",
            "DRAFT",
            "FINAL",
        ]
        | None
    ) = None
    percent_complete: int | None = Field(default=None, ge=0, le=100)
    priority: int | None = Field(default=None, ge=0, le=9)
    categories: list[str] | None = None
    classification: Literal["PUBLIC", "PRIVATE", "CONFIDENTIAL"] | None = None
    transparency: Literal["OPAQUE", "TRANSPARENT"] | None = None
    recurrence_rule: str | None = Field(default=None, max_length=2000)
    recurrence_dates: list[TimeValue] | None = None
    exception_dates: list[TimeValue] | None = None
    related_to: list[Relation] | None = None
    alarms: list[AlarmSpec] | None = None
    attachments: list[Attachment] | None = None


class NewItem(Model):
    kind: ComponentKind
    uid: str = Field(min_length=1, max_length=255, pattern=r"^[^\r\n]+$")
    fields: ItemPatch


class SeriesScope(Model):
    kind: Literal["series"] = "series"


class OccurrenceScope(Model):
    kind: Literal["occurrence"] = "occurrence"
    recurrence_id: TimeValue


Scope = Annotated[SeriesScope | OccurrenceScope, Field(discriminator="kind")]


class Search(Model):
    kind: ComponentKind
    calendars: list[CalendarRef] = Field(min_length=1)
    period: Period | None = None
    text: str | None = None
    completed: bool | None = None
    expand: bool = False
    limit: int = Field(default=100, ge=1, le=500)

    @model_validator(mode="after")
    def expansion_period(self) -> Self:
        if self.completed is not None and self.kind != "VTODO":
            raise ValueError("completed is only valid for tasks")
        if self.expand and self.period is None:
            raise ValueError("Expansion requires a bounded period")
        return self
