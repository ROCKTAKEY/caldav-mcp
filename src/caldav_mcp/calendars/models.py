from typing import Literal

from pydantic import Field, field_validator

from caldav_mcp.contracts import Model
from caldav_mcp.temporal import timezone_name

ComponentKind = Literal["VEVENT", "VTODO", "VJOURNAL"]


class CalendarRef(Model):
    account: str
    url: str


class CalendarProperties(Model):
    timezone: str | None = None

    _timezone = field_validator("timezone")(timezone_name)

    name: str | None = None
    description: str | None = None
    color: str | None = Field(default=None, pattern=r"^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$")


class NewCalendar(Model):
    timezone: str | None = None
    _timezone = field_validator("timezone")(timezone_name)

    account: str
    name: str = Field(min_length=1)
    components: list[ComponentKind] = Field(default=["VEVENT", "VTODO", "VJOURNAL"], min_length=1)
    description: str | None = None
