"""Browser Agent — Playwright-based web scraping with Screenshot-Loop.

FEAT-023 Tunnel Vision mode:
- Uses SessionFingerprint for consistent identity (REGLA-387)
- Emits browser.* events for live streaming (REGLA-390)
- Saves screenshots per session for traceability (REGLA-392/393/394)
- Supports proxy (REGLA-385)

Core rules preserved:
- REGLA-360: Playwright is optional — lazy import, descriptive error if missing.
- REGLA-361: Screenshot-Loop hard limit of 25 steps.
- REGLA-363: Browser always closes (try/finally).
"""

from __future__ import annotations

import base64
import json
import logging
import re
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)

# Hard limit for screenshot-loop steps (REGLA-361)
MAX_AGENT_STEPS = 25

# Screenshot storage base
_SCREENSHOTS_DIR = Path.home() / ".datamirai" / "screenshots"


def _check_playwright() -> None:
    """Raise descriptive error if playwright is not installed. REGLA-360."""
    try:
        import playwright  # noqa: F401
    except ImportError:
        raise RuntimeError(
            "Browser mode requiere Playwright. Instala con: "
            "pip install datamirai-engine[browser] "
            "y luego: playwright install chromium"
        )


async def _emit_event(context: Any, event_type: str, data: dict) -> None:
    """Emit a browser event if the context has an event emitter."""
    emitter = getattr(context, "events", None) if context else None
    session_id = getattr(context, "session_id", None) if context else None
    node_id = getattr(context, "node_id", None) if context else None

    if not emitter or not session_id:
        return

    try:
        from datamirai_engine.core.events import ExecutionEvent, EventType
        type_map = {
            "browser.screenshot": EventType.BROWSER_SCREENSHOT,
            "browser.action": EventType.BROWSER_ACTION,
            "browser.navigation": EventType.BROWSER_NAVIGATION,
            "browser.completed": EventType.BROWSER_COMPLETED,
        }
        et = type_map.get(event_type)
        if et:
            await emitter.emit(ExecutionEvent(
                type=et,
                session_id=session_id,
                node_id=node_id,
                data=data,
            ))
    except Exception:
        pass


def _get_screenshot_dir(session_id: str, run_id: str | None = None) -> Path:
    """Get screenshot directory for a session/run. REGLA-394."""
    if run_id:
        return _SCREENSHOTS_DIR / session_id / run_id
    return _SCREENSHOTS_DIR / session_id


async def fetch_with_browser(
    url: str,
    *,
    timeout: int = 30,
    wait_for_selector: str = "",
    wait_timeout: int = 10,
    stealth: bool = True,
    fingerprint: Any = None,
    proxy: str | None = None,
) -> str:
    """Open URL in headless Playwright, wait for JS, return rendered HTML.

    REGLA-363: Browser always closes via try/finally.
    REGLA-387: Uses SessionFingerprint if provided.
    """
    _check_playwright()
    from playwright.async_api import async_playwright

    async with async_playwright() as p:
        launch_kwargs: dict[str, Any] = {"headless": True}
        if proxy:
            launch_kwargs["proxy"] = {"server": proxy}

        browser = await p.chromium.launch(**launch_kwargs)
        try:
            if fingerprint and stealth:
                ctx_opts = fingerprint.to_playwright_context()
            elif stealth:
                from datamirai_engine.tools.builtin.data.stealth import SessionFingerprint as SF
                ctx_opts = SF.generate().to_playwright_context()
            else:
                ctx_opts = {"viewport": {"width": 1280, "height": 800}}

            browser_context = await browser.new_context(**ctx_opts)
            page = await browser_context.new_page()

            await page.goto(url, wait_until="networkidle", timeout=timeout * 1000)

            if wait_for_selector:
                try:
                    await page.wait_for_selector(
                        wait_for_selector, timeout=wait_timeout * 1000
                    )
                except Exception:
                    logger.warning("Selector '%s' not found within %ds", wait_for_selector, wait_timeout)

            await page.wait_for_timeout(2000)

            html = await page.content()
            return html
        finally:
            await browser.close()


async def agent_scrape(
    url: str,
    objective: str,
    context: Any,
    *,
    max_steps: int = 10,
    model: str = "claude-sonnet",
    save_screenshots: bool = False,
    stealth: bool = True,
    fingerprint: Any = None,
    proxy: str | None = None,
) -> dict[str, Any]:
    """Screenshot-Loop agent: sees the screen, decides, acts, repeats.

    REGLA-361: Hard limit of 25 steps.
    REGLA-362: Each step logged to session trace.
    REGLA-363: Browser always closes.
    REGLA-387: Uses SessionFingerprint for Playwright context.
    REGLA-388: JPEG q70 for streaming, PNG for traceability.
    REGLA-390: Each action emitted as BROWSER_ACTION with reasoning.
    REGLA-392/393/394: Screenshots saved per session with manifest.
    """
    _check_playwright()
    from playwright.async_api import async_playwright
    from datamirai_engine.tools.builtin.data.stealth import apply_jitter

    # Enforce hard limit (REGLA-361)
    max_steps = min(max_steps, MAX_AGENT_STEPS)

    session_id = getattr(context, "session_id", None) if context else None
    run_id = None
    if context and hasattr(context, "run_id"):
        run_id = getattr(context, "run_id", None)

    # Screenshot dir — REGLA-394
    screenshot_dir: Path | None = None
    if save_screenshots and session_id:
        screenshot_dir = _get_screenshot_dir(session_id, run_id)
        screenshot_dir.mkdir(parents=True, exist_ok=True)

    started_at = datetime.now(timezone.utc).isoformat()
    manifest_steps: list[dict[str, Any]] = []
    status = "completed"
    error_msg = ""

    async with async_playwright() as p:
        launch_kwargs: dict[str, Any] = {"headless": True}
        if proxy:
            launch_kwargs["proxy"] = {"server": proxy}

        browser = await p.chromium.launch(**launch_kwargs)
        try:
            # REGLA-387: Use same fingerprint as Windowless
            if fingerprint and stealth:
                ctx_opts = fingerprint.to_playwright_context()
            elif stealth:
                from datamirai_engine.tools.builtin.data.stealth import SessionFingerprint as SF
                fp = SF.generate()
                ctx_opts = fp.to_playwright_context()
            else:
                ctx_opts = {"viewport": {"width": 1280, "height": 800}}

            browser_context = await browser.new_context(**ctx_opts)
            page = await browser_context.new_page()

            await page.goto(url, wait_until="networkidle", timeout=30000)

            # Emit navigation event
            await _emit_event(context, "browser.navigation", {
                "url": url,
                "title": await page.title(),
                "step": 0,
            })

            # Save initial navigation screenshot
            if screenshot_dir:
                nav_screenshot = await page.screenshot(type="png")
                (screenshot_dir / "step-00-navigation.png").write_bytes(nav_screenshot)
                manifest_steps.append({
                    "step": 0,
                    "type": "navigation",
                    "url": url,
                    "screenshot": "step-00-navigation.png",
                    "timestamp": datetime.now(timezone.utc).isoformat(),
                })

            history: list[dict[str, Any]] = []
            collected_data: list[str] = []

            for step in range(max_steps):
                step_num = step + 1

                # 1. Screenshot — PNG for storage, JPEG for streaming (REGLA-388)
                screenshot_png = await page.screenshot(type="png")

                # Emit screenshot via SSE — JPEG q70 for bandwidth (REGLA-388)
                screenshot_jpeg = await page.screenshot(type="jpeg", quality=70)
                screenshot_b64 = base64.b64encode(screenshot_jpeg).decode("ascii")
                await _emit_event(context, "browser.screenshot", {
                    "step": step_num,
                    "url": page.url,
                    "screenshot_base64": screenshot_b64,
                    "format": "jpeg",
                })

                # Save PNG to disk (REGLA-392)
                if screenshot_dir:
                    filename = f"step-{step_num:02d}-screenshot.png"
                    (screenshot_dir / filename).write_bytes(screenshot_png)

                # 2. LLM vision analyzes screenshot + objective
                prompt = _build_agent_prompt(objective, history, step, max_steps)

                response = await context.llm.call(
                    model=model,
                    prompt=prompt,
                    images=[screenshot_png],
                )

                # Parse response
                result_text = response.get("result", "") if isinstance(response, dict) else str(response)
                try:
                    action = json.loads(result_text)
                except json.JSONDecodeError:
                    json_match = re.search(r"\{[^{}]+\}", result_text)
                    if json_match:
                        action = json.loads(json_match.group(0))
                    else:
                        action = {"type": "done", "reasoning": "Could not parse LLM response"}

                # REGLA-362: Log step
                step_log = {"step": step, "action": action}
                history.append(step_log)
                logger.info("Agent step %d: %s", step, action.get("type", "unknown"))

                # Emit action event — REGLA-390
                await _emit_event(context, "browser.action", {
                    "step": step_num,
                    "action_type": action.get("type", "unknown"),
                    "reasoning": action.get("reasoning", ""),
                    "params": {k: v for k, v in action.items() if k not in ("type", "reasoning")},
                })

                # Save action to disk
                if screenshot_dir:
                    action_filename = f"step-{step_num:02d}-action.json"
                    (screenshot_dir / action_filename).write_text(
                        json.dumps(action, ensure_ascii=False, indent=2)
                    )
                    manifest_steps.append({
                        "step": step_num,
                        "action": action,
                        "screenshot": f"step-{step_num:02d}-screenshot.png",
                        "timestamp": datetime.now(timezone.utc).isoformat(),
                    })

                # 3. Execute action
                action_type = action.get("type", "done")

                if action_type == "click":
                    x, y = action.get("x", 0), action.get("y", 0)
                    await page.mouse.click(x, y)
                    await page.wait_for_timeout(int(apply_jitter(1500)))

                elif action_type == "type":
                    x, y = action.get("x", 0), action.get("y", 0)
                    await page.mouse.click(x, y)
                    await page.keyboard.type(action.get("text", ""), delay=50)
                    await page.wait_for_timeout(int(apply_jitter(1000)))

                elif action_type == "scroll":
                    delta = action.get("delta", 300)
                    await page.mouse.wheel(0, delta)
                    await page.wait_for_timeout(int(apply_jitter(1000)))

                elif action_type == "extract":
                    collected_data.append(action.get("data", ""))

                elif action_type == "done":
                    if action.get("data"):
                        collected_data.append(action["data"])
                    break

        except Exception as exc:
            status = "failed"
            error_msg = str(exc)
            raise

        finally:
            await browser.close()

            # Emit completed event
            await _emit_event(context, "browser.completed", {
                "steps": len(history),
                "data_collected": len(collected_data),
                "status": status,
            })

            # Write manifest — REGLA-393: always created
            if screenshot_dir:
                fp_dict = {}
                if fingerprint and hasattr(fingerprint, "to_dict"):
                    fp_dict = fingerprint.to_dict()

                manifest = {
                    "session_id": session_id,
                    "run_id": run_id,
                    "started_at": started_at,
                    "completed_at": datetime.now(timezone.utc).isoformat(),
                    "fingerprint": fp_dict,
                    "initial_url": url,
                    "objective": objective,
                    "total_steps": len(history),
                    "steps": manifest_steps,
                    "data_collected": collected_data,
                    "status": status,
                    "error": error_msg or None,
                }
                (screenshot_dir / "manifest.json").write_text(
                    json.dumps(manifest, ensure_ascii=False, indent=2)
                )

    return {
        "data": collected_data,
        "steps": len(history),
        "history": history,
        "url": url,
    }


def _build_agent_prompt(
    objective: str,
    history: list[dict],
    step: int,
    max_steps: int,
) -> str:
    """Build the prompt for the screenshot-loop LLM."""
    history_text = ""
    if history:
        recent = history[-3:]  # Only last 3 steps for context efficiency
        history_text = "\n".join(
            f"  Step {h['step']}: {json.dumps(h['action'])}" for h in recent
        )

    return f"""Eres un agente de web scraping. Tu objetivo: {objective}

Estas viendo una captura de pantalla de una pagina web.
Paso actual: {step + 1} de {max_steps}

{f'Historial de acciones recientes:\n{history_text}' if history_text else 'Primera accion.'}

Responde SOLO con JSON valido. Elige UNA accion:
- {{"type": "click", "x": <int>, "y": <int>, "reasoning": "<por que>"}}
- {{"type": "type", "x": <int>, "y": <int>, "text": "<texto>", "reasoning": "<por que>"}}
- {{"type": "scroll", "delta": <int>, "reasoning": "<por que>"}}
- {{"type": "extract", "data": "<datos en Markdown>", "reasoning": "<por que>"}}
- {{"type": "done", "data": "<datos finales si los hay>", "reasoning": "<por que termine>"}}

Prioriza eficiencia: extrae lo que necesitas con el minimo de pasos."""
