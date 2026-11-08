"""An in-house inventory API, as a Python module for the importer to read.

The same API the Rust importer's fixture crate declares, so that the two
can be compared: what a `dataclass` becomes, what an `Enum` becomes, what
a `Protocol` becomes, and what has no SysML shape and is listed instead.
"""

from dataclasses import dataclass
from enum import Enum
from typing import Optional, Protocol


class Region(Enum):
    """Where a warehouse is."""

    TOKYO = "tokyo"
    OSAKA = "osaka"
    FUKUOKA = "fukuoka"


@dataclass
class StockQuery:
    """What to look up."""

    sku: str
    warehouses: list[int]
    note: Optional[str]
    # `filter` is a SysML keyword: the writer must quote it
    filter: bool
    # a container in a container has no SysML shape: listed, not folded
    history: list[list[int]]
    # bytes is a Python scalar no SysML one stands for exactly
    raw: bytes


@dataclass
class StockLevel:
    """What a lookup answers."""

    sku: str
    quantity: int
    locations: list[str]
    fill_ratio: float
    pinned: bool
    region: Region | None


class StoreError(Exception):
    """Why a lookup failed."""


class InventoryStore(Protocol):
    """The inventory service every deployment provides."""

    def get_stock(self, query: StockQuery) -> StockLevel:
        """Look one article up."""
        raise StoreError()

    async def watch(self, sku: str) -> StockLevel:
        """Wait for the level of one article to change."""
        ...

    def refresh(self) -> None:
        """Drop caches."""

    def tally(self, counts: dict[str, int]) -> bool:
        """A mapping has no monomorphic SysML shape: listed instead."""
        ...


def default_region() -> Region:
    """The region a query defaults to."""
    return Region.TOKYO


def pick(candidates: list[Region], index) -> Region:
    """An unannotated parameter has no type to write: listed instead."""
    return candidates[index]


Pair = tuple[int, int]
