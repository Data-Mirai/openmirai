"""Scheduler — runs scheduled agents at configured intervals.

Manages background tasks that fire agent executions based on
interval_seconds or cron expressions.
"""

from __future__ import annotations

import asyncio
import logging
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any

logger = logging.getLogger(__name__)


@dataclass
class ScheduledJob:
    agent_id: str
    interval_seconds: float
    cron: str | None = None
    run_count: int = 0
    last_run: float = 0
    _task: asyncio.Task | None = field(default=None, repr=False)


class Scheduler:
    """Background scheduler that fires agent executions on intervals.

    Usage:
        scheduler = Scheduler(on_fire=my_callback)
        scheduler.add("agent-1", interval_seconds=30)
        await scheduler.start()
        # ... runs in background ...
        await scheduler.stop()
    """

    def __init__(
        self,
        on_fire: Callable[[str, int], Awaitable[Any]],
    ) -> None:
        self._on_fire = on_fire
        self._jobs: dict[str, ScheduledJob] = {}
        self._running = False

    def add(
        self,
        agent_id: str,
        interval_seconds: float | None = None,
        cron: str | None = None,
    ) -> None:
        if interval_seconds is None and cron is None:
            raise ValueError("Either interval_seconds or cron must be specified")

        # For cron, convert to approximate interval (basic support)
        effective_interval = interval_seconds or self._cron_to_seconds(cron)

        job = ScheduledJob(
            agent_id=agent_id,
            interval_seconds=effective_interval,
            cron=cron,
        )
        self._jobs[agent_id] = job
        logger.info("Scheduled agent '%s' every %ss", agent_id, effective_interval)

        # If already running, start the job immediately
        if self._running:
            job._task = asyncio.create_task(self._run_job(job))

    def remove(self, agent_id: str) -> None:
        job = self._jobs.pop(agent_id, None)
        if job and job._task:
            job._task.cancel()
            logger.info("Unscheduled agent '%s'", agent_id)

    async def start(self) -> None:
        self._running = True
        for job in self._jobs.values():
            if job._task is None:
                job._task = asyncio.create_task(self._run_job(job))
        logger.info("Scheduler started with %d jobs", len(self._jobs))

    async def stop(self) -> None:
        self._running = False
        for job in self._jobs.values():
            if job._task:
                job._task.cancel()
                job._task = None
        logger.info("Scheduler stopped")

    def list_jobs(self) -> list[dict[str, Any]]:
        return [
            {
                "agent_id": j.agent_id,
                "interval_seconds": j.interval_seconds,
                "cron": j.cron,
                "run_count": j.run_count,
                "last_run": j.last_run,
            }
            for j in self._jobs.values()
        ]

    async def _run_job(self, job: ScheduledJob) -> None:
        # Wait initial interval before first run
        await asyncio.sleep(job.interval_seconds)

        while self._running:
            try:
                job.run_count += 1
                job.last_run = time.time()
                logger.info(
                    "Firing agent '%s' (run #%d)", job.agent_id, job.run_count
                )
                await self._on_fire(job.agent_id, job.run_count)
            except asyncio.CancelledError:
                break
            except Exception:
                logger.exception("Error firing agent '%s'", job.agent_id)

            try:
                await asyncio.sleep(job.interval_seconds)
            except asyncio.CancelledError:
                break

    @staticmethod
    def _cron_to_seconds(cron: str | None) -> float:
        """Basic cron-to-interval approximation for common patterns."""
        if not cron:
            return 60
        parts = cron.strip().split()
        if len(parts) < 5:
            return 60

        minute, hour, dom, _month, _dow = parts[:5]

        # Every minute: * * * * *
        if all(p == "*" for p in parts[:5]):
            return 60
        # Every N minutes: */N * * * *
        if minute.startswith("*/"):
            return int(minute[2:]) * 60
        # Hourly: 0 * * * *
        if minute != "*" and hour == "*":
            return 3600
        # Daily: 0 0 * * *
        if minute != "*" and hour != "*" and dom == "*":
            return 86400

        return 3600  # default: hourly
