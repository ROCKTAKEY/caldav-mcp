from datetime import UTC, date, datetime
from typing import Annotated, Literal, Self
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from pydantic import AwareDatetime, Field, NaiveDatetime, model_validator

from caldav_mcp.contracts import Model


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


class Period(Model):
    start: AwareDatetime
    end: AwareDatetime

    @model_validator(mode="after")
    def ordered(self) -> Self:
        if self.end <= self.start:
            raise ValueError("end must be after start")
        return self


def timezone_name(value: str | None) -> str | None:
    if value is not None:
        try:
            ZoneInfo(value)
        except (ValueError, ZoneInfoNotFoundError) as exc:
            raise ValueError("Unknown timezone") from exc
    return value
