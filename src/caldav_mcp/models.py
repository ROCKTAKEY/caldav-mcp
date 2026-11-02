from datetime import UTC, date, datetime, timedelta
from typing import Annotated, Literal, Self
from urllib.parse import urlsplit
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from pydantic import (
    AwareDatetime,
    BaseModel,
    ConfigDict,
    Field,
    NaiveDatetime,
    field_validator,
    model_validator,
)


class Model(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class DateValue(Model):
    kind: Literal["date"] = "date"
    value: date

    def native(self) -> date:
        return self.value


class FloatingValue(Model):
    kind: Literal["floating"] = "floating"
    value: NaiveDatetime

    def native(self) -> datetime:
        return self.value


class UtcValue(Model):
    kind: Literal["utc"] = "utc"
    value: AwareDatetime

    def native(self) -> datetime:
        return self.value.astimezone(UTC)


class ZonedValue(Model):
    kind: Literal["zoned"] = "zoned"
    value: NaiveDatetime
    timezone: str

    @model_validator(mode="after")
    def unambiguous(self) -> Self:
        try:
            zone = ZoneInfo(self.timezone)
        except (ZoneInfoNotFoundError, ValueError) as exc:
            raise ValueError("Unknown IANA timezone") from exc
        local = self.value.replace(tzinfo=zone)
        if local.astimezone(UTC).astimezone(zone).replace(tzinfo=None) != self.value:
            raise ValueError("Local time does not exist; choose a valid local time")
        if local.utcoffset() != local.replace(fold=1).utcoffset():
            raise ValueError("Local time is ambiguous; specify a UTC instant instead")
        return self

    def native(self) -> datetime:
        return self.value.replace(tzinfo=ZoneInfo(self.timezone))


TimeValue = Annotated[
    DateValue | FloatingValue | UtcValue | ZonedValue, Field(discriminator="kind")
]
ComponentKind = Literal["VEVENT", "VTODO", "VJOURNAL"]


class Period(Model):
    start: AwareDatetime
    end: AwareDatetime

    @model_validator(mode="after")
    def ordered(self) -> Self:
        if self.end <= self.start:
            raise ValueError("end must be after start")
        return self


def http_url(value: str) -> str:
    parsed = urlsplit(value)
    if parsed.scheme not in ("http", "https") or not parsed.hostname:
        raise ValueError("Expected an absolute HTTP(S) URL")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("URL must not contain credentials, query, or fragment")
    return value


class AccountConfig(Model):
    id: str = Field(pattern=r"^[a-zA-Z0-9_-]+$")
    url: str
    username: str | None = None
    password_env: str | None = None
    bearer_token_env: str | None = None
    calendars: tuple[str, ...] = ()
    sharing_url: str | None = None
    default_calendar: str | None = None
    timeout_seconds: float = Field(default=30, gt=0, le=300)

    _url = field_validator("url")(http_url)

    @model_validator(mode="after")
    def authentication(self) -> Self:
        if self.bearer_token_env and (self.username or self.password_env):
            raise ValueError("Choose basic or bearer authentication")
        if self.password_env and not self.username:
            raise ValueError("password_env requires username")
        for url in (*self.calendars, self.sharing_url, self.default_calendar):
            if url is not None:
                http_url(url)
                if (
                    urlsplit(url).netloc != urlsplit(self.url).netloc
                    or urlsplit(url).scheme != urlsplit(self.url).scheme
                ):
                    raise ValueError("Account URLs must have the same origin")
        return self


class Settings(Model):
    accounts: tuple[AccountConfig, ...] = Field(min_length=1)
    timezone: str = "UTC"

    @model_validator(mode="after")
    def unique(self) -> Self:
        if len({a.id for a in self.accounts}) != len(self.accounts):
            raise ValueError("Account IDs must be unique")
        try:
            ZoneInfo(self.timezone)
        except (ValueError, ZoneInfoNotFoundError) as exc:
            raise ValueError("Unknown timezone") from exc
        return self


class CalendarRef(Model):
    account: str
    url: str


class ResourceRef(Model):
    calendar: CalendarRef
    url: str


class Revision(Model):
    etag: str = Field(pattern=r'^"[^"\r\n]*"$')


class AlarmSpec(Model):
    description: str
    trigger: timedelta
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
    expand: bool = False
    limit: int = Field(default=100, ge=1, le=500)

    @model_validator(mode="after")
    def expansion_period(self) -> Self:
        if self.expand and self.period is None:
            raise ValueError("Expansion requires a bounded period")
        return self


class CalendarProperties(Model):
    name: str | None = None
    description: str | None = None
    color: str | None = Field(default=None, pattern=r"^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$")


class NewCalendar(Model):
    account: str
    name: str = Field(min_length=1)
    components: list[ComponentKind] = Field(default=["VEVENT", "VTODO", "VJOURNAL"], min_length=1)
    description: str | None = None
