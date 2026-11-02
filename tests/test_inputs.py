from datetime import date

import pytest
from pydantic import ValidationError

from caldav_mcp.models import AccountConfig, DateValue, FloatingValue, Period, ZonedValue


def test_calendar_dates_keep_their_meaning():
    assert DateValue(value="2026-11-02").native() == date(2026, 11, 2)
    assert FloatingValue(value="2026-11-02T10:00:00").native().tzinfo is None
    assert (
        ZonedValue(value="2026-11-02T10:00:00", timezone="Asia/Tokyo")
        .native()
        .utcoffset()
        .total_seconds()
        == 32400
    )


def test_invalid_or_ambiguous_local_times_are_not_guessed():
    with pytest.raises(ValidationError):
        ZonedValue(value="2026-03-08T02:30:00", timezone="America/New_York")
    with pytest.raises(ValidationError):
        ZonedValue(value="2026-11-01T01:30:00", timezone="America/New_York")
    with pytest.raises(ValidationError):
        FloatingValue(value="2026-11-02T10:00:00Z")


def test_query_period_is_aware_and_nonempty():
    with pytest.raises(ValidationError):
        Period(start="2026-11-02T10:00:00", end="2026-11-03T10:00:00")
    with pytest.raises(ValidationError):
        Period(start="2026-11-03T10:00:00Z", end="2026-11-02T10:00:00Z")


def test_credentials_are_environment_references_not_urls():
    with pytest.raises(ValidationError):
        AccountConfig(id="work", url="https://alice:secret@example.org/dav/")
