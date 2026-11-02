from typing import Self
from urllib.parse import urlsplit
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from pydantic import Field, field_validator, model_validator

from caldav_mcp.contracts import Model


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
