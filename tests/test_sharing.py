from uuid import uuid4

import pytest

from caldav_mcp.calendars.dav import Calendars
from caldav_mcp.calendars.models import CalendarRef, NewCalendar
from caldav_mcp.dav import DavClient
from caldav_mcp.errors import CalendarError
from caldav_mcp.items.dav import Items
from caldav_mcp.items.models import ItemPatch, NewItem, Search
from caldav_mcp.sharing import Sharing


@pytest.mark.asyncio
async def test_radicale_share_accept_read_write_revoke(settings):
    owner_client, recipient_client = [DavClient(a) for a in settings.accounts]
    owner, recipient = Calendars(owner_client), Calendars(recipient_client)
    owner_shares, recipient_shares = Sharing(owner), Sharing(recipient)
    try:
        source = await owner.create(NewCalendar(account="alice", name="Shared"))
        await Items(owner).create(
            source.ref,
            NewItem(kind="VTODO", uid=str(uuid4()), fields=ItemPatch(summary="Shared task")),
        )
        path = f"/bob/shared-{uuid4()}/"
        await owner_shares.create(source.ref, "bob", path, "read")
        invitations = await recipient_shares.list()
        assert any(s["path"] == path for s in invitations)
        await recipient_shares.update(path, enabled=True, hidden=False)
        shared = CalendarRef(account="bob", url=settings.accounts[1].url.rstrip("/") + path)
        assert shared in [c.ref for c in await recipient.list()]
        query = Search(kind="VTODO", calendars=[shared])
        assert len((await Items(recipient).search(shared, query))["items"]) == 1
        spec = NewItem(kind="VTODO", uid=str(uuid4()), fields=ItemPatch(summary="Added by Bob"))
        with pytest.raises(CalendarError) as denied:
            await Items(recipient).create(shared, spec)
        assert denied.value.code == "forbidden"
        await owner_shares.update(path, access="read_write")
        await Items(recipient).create(shared, spec)
        assert (
            len(
                (
                    await Items(owner).search(
                        source.ref, Search(kind="VTODO", calendars=[source.ref])
                    )
                )["items"]
            )
            == 2
        )
        await owner_shares.delete(path)
        with pytest.raises(CalendarError):
            await recipient.get(shared)
        await owner.delete(source.ref)
    finally:
        await owner_client.close()
        await recipient_client.close()
