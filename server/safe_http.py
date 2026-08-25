"""Bounded outbound HTTP helpers for user-configured sources and actions."""

from __future__ import annotations

import asyncio
import ipaddress
import socket
from collections.abc import Iterable
from urllib.parse import urljoin, urlsplit

from aiohttp import ClientSession


class UnsafeUrlError(ValueError):
    pass


class ResponseTooLargeError(ValueError):
    pass


def parse_remote_url(url: str) -> tuple[str, int]:
    try:
        parsed = urlsplit(url)
        port = parsed.port
    except ValueError as exc:
        raise UnsafeUrlError("invalid URL") from exc
    if parsed.scheme not in {"http", "https"}:
        raise UnsafeUrlError("only http and https URLs are allowed")
    if not parsed.hostname:
        raise UnsafeUrlError("URL must include a host")
    if parsed.username or parsed.password:
        raise UnsafeUrlError("URL credentials are not allowed")
    return parsed.hostname.rstrip(".").lower(), port or (443 if parsed.scheme == "https" else 80)


def _address_allowed(address: str, allowed_hosts: set[str], hostname: str) -> bool:
    if hostname in allowed_hosts or address in allowed_hosts:
        return True
    try:
        return ipaddress.ip_address(address).is_global
    except ValueError:
        return False


async def validate_remote_url(url: str, allowed_hosts: Iterable[str] = ()) -> None:
    hostname, port = parse_remote_url(url)
    allowed = {entry.rstrip(".").lower() for entry in allowed_hosts if entry}
    if hostname in allowed:
        return
    try:
        literal_address = ipaddress.ip_address(hostname)
    except ValueError:
        literal_address = None
    if literal_address is not None:
        if not literal_address.is_global:
            raise UnsafeUrlError("private, local, link-local and reserved network addresses are blocked")
        return
    try:
        infos = await asyncio.wait_for(
            asyncio.to_thread(socket.getaddrinfo, hostname, port, type=socket.SOCK_STREAM),
            timeout=5,
        )
    except TimeoutError as exc:
        raise UnsafeUrlError(f"host resolution timed out: {hostname}") from exc
    except socket.gaierror as exc:
        raise UnsafeUrlError(f"host cannot be resolved: {hostname}") from exc
    addresses = {info[4][0] for info in infos}
    if not addresses or any(not _address_allowed(address, allowed, hostname) for address in addresses):
        raise UnsafeUrlError("private, local, link-local and reserved network addresses are blocked")


async def fetch_bytes(
    session: ClientSession,
    url: str,
    *,
    max_bytes: int,
    allowed_hosts: Iterable[str] = (),
    method: str = "GET",
    body: str = "",
    max_redirects: int = 3,
) -> tuple[bytes, str]:
    current = url
    for redirect_count in range(max_redirects + 1):
        await validate_remote_url(current, allowed_hosts)
        async with session.request(
            method,
            current,
            data=body.encode("utf-8") if method == "POST" else None,
            allow_redirects=False,
        ) as response:
            if response.status in {301, 302, 303, 307, 308}:
                if redirect_count >= max_redirects:
                    raise UnsafeUrlError("too many redirects")
                location = response.headers.get("Location")
                if not location:
                    raise UnsafeUrlError("redirect response has no Location")
                current = urljoin(current, location)
                if response.status == 303:
                    method, body = "GET", ""
                continue
            response.raise_for_status()
            content_length = response.content_length
            if content_length is not None and content_length > max_bytes:
                raise ResponseTooLargeError(f"response exceeds {max_bytes} bytes")
            chunks = bytearray()
            async for chunk in response.content.iter_chunked(min(16384, max_bytes + 1)):
                chunks.extend(chunk)
                if len(chunks) > max_bytes:
                    raise ResponseTooLargeError(f"response exceeds {max_bytes} bytes")
            return bytes(chunks), str(response.url)
    raise UnsafeUrlError("too many redirects")
