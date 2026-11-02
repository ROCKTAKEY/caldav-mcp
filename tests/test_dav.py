import httpx
import pytest

from caldav_mcp.dav import C, D, DavClient, properties
from caldav_mcp.errors import CalendarError
from caldav_mcp.models import AccountConfig


@pytest.mark.asyncio
async def test_credentials_never_follow_foreign_hrefs():
    client = DavClient(AccountConfig(id="a", url="https://dav.test/"))
    try:
        with pytest.raises(CalendarError, match="origin"):
            client.resolve("https://evil.test/steal")
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_conditional_write_conflict_is_structured():
    client = DavClient(
        AccountConfig(id="a", url="https://dav.test/"),
        httpx.MockTransport(lambda r: httpx.Response(412)),
    )
    try:
        with pytest.raises(CalendarError) as caught:
            await client.request("PUT", "https://dav.test/a.ics", content=b"ics")
        assert caught.value.code == "conflict"
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_lost_write_response_is_not_retried():
    calls = []

    def fail(request):
        calls.append(request)
        raise httpx.ReadTimeout("response lost")

    client = DavClient(AccountConfig(id="a", url="https://dav.test/"), httpx.MockTransport(fail))
    try:
        with pytest.raises(CalendarError) as caught:
            await client.request("PUT", "https://dav.test/a.ics", content=b"ics")
        assert caught.value.code == "outcome_unknown"
        assert len(calls) == 1
    finally:
        await client.close()


def test_multistatus_preserves_failed_properties():
    body = b"""<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
    <d:response><d:href>/a/</d:href>
    <d:propstat><d:prop><d:displayname>A</d:displayname></d:prop>
    <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
    <d:propstat><d:prop><c:supported-calendar-component-set/></d:prop>
    <d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
    </d:response></d:multistatus>"""
    row = properties(body)[0]
    assert row.values[f"{{{D}}}displayname"].text == "A"
    assert row.failed[f"{{{C}}}supported-calendar-component-set"] == 404


def test_multistatus_cannot_expand_xml_entities():
    with pytest.raises(CalendarError):
        properties(b'<!DOCTYPE x [<!ENTITY a "boom">]><x>&a;</x>')
