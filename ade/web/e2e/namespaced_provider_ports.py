"""Loopback-port reservations for the namespaced provider fixture.

This Linux-only fixture requires readable `/proc/net/tcp` and IPv4 TCP
listeners reachable through loopback. It cannot pass listening sockets to its engine, Console, or Vite
child, so it cannot make the release-to-bind handoff atomic. Keeping a
reservation until each owning child starts eliminates duplicate selections
within one fixture attempt and lets setup detect and retry a demonstrated
dynamic-port collision. A listener ownership check is performed before every
readiness connection, but it is detection rather than socket handoff; the
launcher deliberately does not claim an impossible atomic reservation.
"""
from __future__ import annotations

import errno
import os
import platform
import socket
from collections.abc import Mapping, Sequence
from dataclasses import dataclass


@dataclass
class LoopbackPortReservation:
    """Hold one loopback TCP port until its configured child is about to start.

    `configured` distinguishes an operator-provided port from an ephemeral
    choice. The launcher may retry only the latter after an address-in-use
    failure; retrying an explicit port would hide a broken fixture contract.
    """

    name: str
    port: int
    configured: bool
    _socket: socket.socket | None

    def release(self) -> None:
        """Release the reservation immediately before spawning its listener."""
        if self._socket is not None:
            self._socket.close()
            self._socket = None


def require_linux_ipv4_listener_support() -> None:
    """Fail before reservations unless `/proc` can verify IPv4 listener owners.

    The fixture is intentionally limited to Linux CI hosts with readable
    `/proc/net/tcp` and IPv4 TCP listeners reachable through loopback. It
    rejects other platforms before reserving a port or spawning a child instead
    of timing out
    later while inspecting a listener table that cannot represent the fixture.
    """
    proc_net_tcp = "/proc/net/tcp"
    requirement = "Linux with readable /proc/net/tcp and IPv4 TCP listeners reachable through loopback"
    if platform.system() != "Linux":
        raise RuntimeError(f"namespaced provider fixture requires {requirement}")
    if not os.path.exists(proc_net_tcp):
        raise RuntimeError(f"namespaced provider fixture requires {requirement}")
    try:
        with open(proc_net_tcp, encoding="utf-8"):
            pass
    except OSError as error:
        raise RuntimeError(f"namespaced provider fixture requires {requirement}") from error


def reserve_loopback_port(
    name: str,
    environment: Mapping[str, str],
    fallback: int = 0,
) -> LoopbackPortReservation:
    """Reserve a configured or ephemeral loopback port without probing a service.

    A configured value is validated and bound now, so an occupied explicit port
    fails before the fixture starts children or sends requests to that address.
    Dynamic reservations remain open until their owner starts, making each
    automatically chosen port distinct inside one fixture attempt.
    """
    raw_value = environment.get(name)
    configured = raw_value is not None
    if configured:
        try:
            requested_port = int(raw_value)
        except ValueError as error:
            raise RuntimeError(f"{name} must be an integer TCP port") from error
        if not 1 <= requested_port <= 65535:
            raise RuntimeError(f"{name} must be between 1 and 65535")
    else:
        requested_port = fallback

    reservation = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        reservation.bind(("127.0.0.1", requested_port))
    except OSError as error:
        reservation.close()
        if configured:
            raise RuntimeError(
                f"{name}={requested_port} is unavailable: {error}"
            ) from error
        raise RuntimeError(f"could not reserve a loopback port for {name}: {error}") from error

    return LoopbackPortReservation(
        name=name,
        port=int(reservation.getsockname()[1]),
        configured=configured,
        _socket=reservation,
    )


def release_reservations(reservations: Sequence[LoopbackPortReservation]) -> None:
    """Close every still-held reservation during launcher cleanup or retry."""
    for reservation in reservations:
        reservation.release()


def listener_owner_pids(port: int) -> set[int]:
    """Return PIDs that own listening loopback sockets for `port` on Linux.

    The launcher uses this before a readiness connection to reject a detected
    unrelated process that won a release-to-bind race. Linux CI exposes socket
    inodes through `/proc`; other platforms fail closed rather than weakening
    that ownership check. This is detection, not an atomic socket handoff.
    """
    proc_net_tcp = "/proc/net/tcp"
    if not os.path.exists(proc_net_tcp):
        raise RuntimeError("listener ownership verification requires Linux /proc")

    port_hex = f"{port:04X}"
    inodes: set[str] = set()
    try:
        with open(proc_net_tcp, encoding="utf-8") as tcp_table:
            lines = tcp_table.read().splitlines()[1:]
    except OSError as error:
        raise RuntimeError("could not inspect listening sockets") from error
    for line in lines:
        fields = line.split()
        if len(fields) < 10 or fields[1].split(":")[-1] != port_hex:
            continue
        if fields[3] == "0A":
            inodes.add(fields[9])

    owners: set[int] = set()
    for proc in os.scandir("/proc"):
        if not proc.name.isdigit() or not proc.is_dir():
            continue
        fd_dir = os.path.join(proc.path, "fd")
        try:
            entries = os.scandir(fd_dir)
        except OSError:
            continue
        with entries:
            for fd in entries:
                try:
                    target = os.readlink(fd.path)
                except OSError:
                    continue
                if target.startswith("socket:[") and target[8:-1] in inodes:
                    owners.add(int(proc.name))
                    break
    return owners


def listener_is_owned_by_process_group(port: int, leader_pid: int) -> bool:
    """Whether a listener belongs to the isolated process group of a child."""
    try:
        process_group = os.getpgid(leader_pid)
    except ProcessLookupError:
        return False
    for owner_pid in listener_owner_pids(port):
        try:
            if os.getpgid(owner_pid) == process_group:
                return True
        except ProcessLookupError:
            continue
    return False


def is_address_in_use(error: BaseException | str) -> bool:
    """Recognize a bind collision from native errors or a child-process log tail."""
    if isinstance(error, OSError) and error.errno == errno.EADDRINUSE:
        return True
    text = str(error).lower()
    return "address already in use" in text or "eaddrinuse" in text
