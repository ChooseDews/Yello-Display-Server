"""Small, server-side-only Home Assistant REST client."""

from __future__ import annotations

import re
from urllib.parse import quote, urlsplit

from aiohttp import ClientSession


ENTITY_ID_RE = re.compile(r"^[a-z0-9_]+\.[a-z0-9_]+$")
SAFE_TOGGLE_DOMAINS = {"light", "switch", "input_boolean", "fan"}


class HomeAssistantError(RuntimeError):
    pass


class HomeAssistantClient:
    def __init__(self, url: str = "", token: str = ""):
        self.url = url.rstrip("/")
        self.token = token

    @property
    def configured(self) -> bool:
        if not self.url or not self.token:
            return False
        parsed = urlsplit(self.url)
        return parsed.scheme in {"http", "https"} and bool(parsed.hostname) and not parsed.username and not parsed.password

    def _headers(self) -> dict[str, str]:
        if not self.configured:
            raise HomeAssistantError("Home Assistant is not configured")
        return {"Authorization": f"Bearer {self.token}", "Content-Type": "application/json"}

    @staticmethod
    def validate_entity_id(entity_id: str) -> str:
        if not ENTITY_ID_RE.fullmatch(entity_id):
            raise HomeAssistantError("invalid Home Assistant entity ID")
        return entity_id

    async def get_state(self, session: ClientSession, entity_id: str) -> dict:
        entity_id = self.validate_entity_id(entity_id)
        path = quote(entity_id, safe="._")
        async with session.get(f"{self.url}/api/states/{path}", headers=self._headers()) as response:
            if response.status == 404:
                raise HomeAssistantError(f"entity not found: {entity_id}")
            response.raise_for_status()
            value = await response.json()
        return self._sanitize_state(value)

    async def list_states(self, session: ClientSession) -> list[dict]:
        async with session.get(f"{self.url}/api/states", headers=self._headers()) as response:
            response.raise_for_status()
            values = await response.json()
        if not isinstance(values, list):
            raise HomeAssistantError("Home Assistant returned an invalid state list")
        return [self._sanitize_state(value) for value in values if isinstance(value, dict)]

    async def toggle(self, session: ClientSession, entity_id: str) -> None:
        entity_id = self.validate_entity_id(entity_id)
        if entity_id.partition(".")[0] not in SAFE_TOGGLE_DOMAINS:
            raise HomeAssistantError("only lights, switches, input booleans, and fans may be toggled")
        async with session.post(
            f"{self.url}/api/services/homeassistant/toggle",
            headers=self._headers(),
            json={"entity_id": entity_id},
        ) as response:
            response.raise_for_status()
            await response.read()

    @staticmethod
    def _sanitize_state(value: dict) -> dict:
        attributes = value.get("attributes") if isinstance(value.get("attributes"), dict) else {}
        # Only renderer/editor-useful values leave the integration boundary.
        safe_attributes = {
            str(key): item
            for key, item in attributes.items()
            if isinstance(item, (str, int, float, bool)) or item is None
        }
        return {
            "entity_id": str(value.get("entity_id", "")),
            "state": str(value.get("state", "unknown")),
            "attributes": safe_attributes,
            "last_changed": str(value.get("last_changed", "")),
        }
