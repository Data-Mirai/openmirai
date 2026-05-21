"""Ollama adapter — local LLM inference via HTTP API."""

from __future__ import annotations

import re
from typing import Any

import httpx

from datamirai_engine.llm.adapter import (
    LLMAdapter,
    ModelInfo,
    NormalizedResponse,
    ToolCall,
)


class OllamaAdapter(LLMAdapter):
    """Adapter for Ollama running locally (or on a custom host)."""

    provider_name = "ollama"

    def __init__(
        self,
        *,
        base_url: str = "http://localhost:11434",
        timeout: float = 60.0,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    # ------------------------------------------------------------------
    # Core interface
    # ------------------------------------------------------------------

    async def call(
        self,
        *,
        model: str,
        prompt: str,
        context: str | None = None,
        temperature: float = 0.7,
        max_tokens: int = 1024,
        **kwargs: Any,
    ) -> NormalizedResponse:
        messages: list[dict[str, str]] = []
        if context:
            messages.append({"role": "system", "content": context})
        messages.append({"role": "user", "content": prompt})

        payload: dict[str, Any] = {
            "model": model,
            "messages": messages,
            "stream": False,
            "options": {
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        }

        num_ctx = kwargs.get("num_ctx")
        if num_ctx:
            payload["options"]["num_ctx"] = int(num_ctx)

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(f"{self.base_url}/api/chat", json=payload)
            resp.raise_for_status()
            data = resp.json()

        raw_text: str = data.get("message", {}).get("content", "")
        cleaned = self._clean_response(raw_text)

        tokens_input = data.get("prompt_eval_count", 0)
        tokens_output = data.get("eval_count", 0)

        return NormalizedResponse(
            response=cleaned,
            tokens_used={"input": tokens_input, "output": tokens_output},
            model=data.get("model", model),
            provider=self.provider_name,
        )

    @staticmethod
    def _convert_messages_for_ollama(messages: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Convert OpenAI-format messages to Ollama-compatible format.

        Ollama differences:
        - assistant tool_calls: no ``id``/``type``, arguments is a dict (not JSON string)
        - tool results: no ``tool_call_id``
        """
        import json as _json

        converted: list[dict[str, Any]] = []
        for msg in messages:
            role = msg.get("role", "")

            if role == "assistant" and "tool_calls" in msg:
                ollama_tcs = []
                for tc in msg["tool_calls"]:
                    fn = tc.get("function", tc)
                    args = fn.get("arguments", {})
                    if isinstance(args, str):
                        try:
                            args = _json.loads(args)
                        except _json.JSONDecodeError:
                            args = {}
                    ollama_tcs.append({"function": {"name": fn["name"], "arguments": args}})
                converted.append({
                    "role": "assistant",
                    "content": msg.get("content") or "",
                    "tool_calls": ollama_tcs,
                })

            elif role == "tool":
                converted.append({
                    "role": "tool",
                    "content": msg.get("content", ""),
                })

            else:
                # system, user — pass through
                converted.append({k: v for k, v in msg.items() if v is not None})

        return converted

    async def call_with_messages(
        self,
        *,
        model: str,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        temperature: float = 0.7,
        max_tokens: int = 4096,
        **kwargs: Any,
    ) -> NormalizedResponse:
        import json

        ollama_messages = self._convert_messages_for_ollama(messages)

        payload: dict[str, Any] = {
            "model": model,
            "messages": ollama_messages,
            "stream": False,
            "options": {
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        }
        if tools:
            payload["tools"] = tools

        num_ctx = kwargs.get("num_ctx")
        if num_ctx:
            payload["options"]["num_ctx"] = int(num_ctx)

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(f"{self.base_url}/api/chat", json=payload)
            resp.raise_for_status()
            data = resp.json()

        message = data.get("message", {})
        raw_text: str = message.get("content", "")
        cleaned = self._clean_response(raw_text)

        tool_calls: list[ToolCall] = []
        for i, tc in enumerate(message.get("tool_calls", [])):
            fn = tc.get("function", {})
            args = fn.get("arguments", {})
            tool_calls.append(
                ToolCall(
                    id=f"call_{i}",
                    name=fn.get("name", ""),
                    arguments=json.dumps(args) if isinstance(args, dict) else str(args),
                )
            )

        return NormalizedResponse(
            response=cleaned,
            tokens_used={
                "input": data.get("prompt_eval_count", 0),
                "output": data.get("eval_count", 0),
            },
            model=data.get("model", model),
            provider=self.provider_name,
            tool_calls=tool_calls,
        )

    async def stream_with_messages(
        self,
        *,
        model: str,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        temperature: float = 0.7,
        max_tokens: int = 4096,
        on_token: Any = None,
        **kwargs: Any,
    ) -> NormalizedResponse:
        """Stream Ollama response token by token, calling on_token for each chunk."""
        import json

        ollama_messages = self._convert_messages_for_ollama(messages)

        payload: dict[str, Any] = {
            "model": model,
            "messages": ollama_messages,
            "stream": True,
            "options": {
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        }
        if tools:
            payload["tools"] = tools

        num_ctx = kwargs.get("num_ctx")
        if num_ctx:
            payload["options"]["num_ctx"] = int(num_ctx)

        full_content = ""
        tool_calls: list[ToolCall] = []
        tokens_input = 0
        tokens_output = 0
        model_used = model

        # Streaming needs longer timeouts: connect fast, but read can be slow per token
        stream_timeout = httpx.Timeout(connect=30.0, read=600.0, write=30.0, pool=30.0)
        async with httpx.AsyncClient(timeout=stream_timeout) as client:
            async with client.stream("POST", f"{self.base_url}/api/chat", json=payload) as resp:
                resp.raise_for_status()
                async for line in resp.aiter_lines():
                    if not line.strip():
                        continue
                    try:
                        chunk = json.loads(line)
                    except json.JSONDecodeError:
                        continue

                    message = chunk.get("message", {})
                    delta = message.get("content", "")

                    if delta and on_token:
                        on_token(delta)
                    full_content += delta

                    # Tool calls can arrive in ANY chunk (not just done=True)
                    for tc in message.get("tool_calls", []):
                        fn = tc.get("function", {})
                        args = fn.get("arguments", {})
                        tc_id = tc.get("id", f"call_{len(tool_calls)}")
                        tool_calls.append(
                            ToolCall(
                                id=tc_id,
                                name=fn.get("name", ""),
                                arguments=json.dumps(args) if isinstance(args, dict) else str(args),
                            )
                        )

                    if chunk.get("done", False):
                        tokens_input = chunk.get("prompt_eval_count", 0)
                        tokens_output = chunk.get("eval_count", 0)
                        model_used = chunk.get("model", model)

        cleaned = self._clean_response(full_content)

        return NormalizedResponse(
            response=cleaned,
            tokens_used={"input": tokens_input, "output": tokens_output},
            model=model_used,
            provider=self.provider_name,
            tool_calls=tool_calls,
        )

    async def embed(self, text: str, *, model: str | None = None) -> list[float]:
        embed_model = model or "nomic-embed-text"
        payload = {"model": embed_model, "prompt": text}

        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(f"{self.base_url}/api/embeddings", json=payload)
            resp.raise_for_status()
            data = resp.json()

        return data.get("embedding", [])

    async def list_models(self) -> list[ModelInfo]:
        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.get(f"{self.base_url}/api/tags")
            resp.raise_for_status()
            data = resp.json()

        models: list[ModelInfo] = []
        for m in data.get("models", []):
            model_name = m.get("name", "")
            ctx_len = await self._get_model_context_length(model_name)
            models.append(
                ModelInfo(
                    id=model_name,
                    name=model_name,
                    context_window=ctx_len,
                    supports_streaming=True,
                    supports_tools=False,
                )
            )
        return models

    async def _get_model_context_length(self, model_name: str) -> int | None:
        """Get context_length for a model by calling /api/show."""
        try:
            async with httpx.AsyncClient(timeout=self.timeout) as client:
                resp = await client.post(
                    f"{self.base_url}/api/show",
                    json={"name": model_name},
                )
                resp.raise_for_status()
                data = resp.json()

            model_info = data.get("model_info", {})
            for key, value in model_info.items():
                if key.endswith(".context_length") and isinstance(value, int):
                    return value
            return None
        except Exception:
            return None

    async def get_model_info(self, model: str) -> dict[str, Any]:
        """Get detailed model info including context_window."""
        async with httpx.AsyncClient(timeout=self.timeout) as client:
            resp = await client.post(
                f"{self.base_url}/api/show",
                json={"name": model},
            )
            resp.raise_for_status()
            data = resp.json()

        details = data.get("details", {})
        model_info = data.get("model_info", {})

        context_window: int | None = None
        embedding_length: int | None = None
        for key, value in model_info.items():
            if key.endswith(".context_length") and isinstance(value, int):
                context_window = value
            if key.endswith(".embedding_length") and isinstance(value, int):
                embedding_length = value

        return {
            "name": model,
            "parameter_size": details.get("parameter_size"),
            "quantization_level": details.get("quantization_level"),
            "family": details.get("family"),
            "context_window": context_window,
            "embedding_length": embedding_length,
        }

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    # Matches innermost <think>...</think> (content must NOT contain <think>)
    _THINK_INNER_RE = re.compile(r"<think>(?:(?!<think>)[\s\S])*?</think>", re.DOTALL)
    _THINK_OPEN_RE = re.compile(r"<think>[\s\S]*$", re.DOTALL)

    @classmethod
    def _clean_response(cls, text: str) -> str:
        """Strip ``<think>...</think>`` blocks from the response (REGLA-33).

        Handles:
        - Normal paired tags: ``<think>reasoning</think>``
        - Nested tags: ``<think>...<think>inner</think>...</think>``
        - Malformed / unclosed: ``<think>reasoning without closing tag``
        """
        # Iteratively remove innermost <think> blocks to handle nesting
        cleaned = text
        while cls._THINK_INNER_RE.search(cleaned):
            cleaned = cls._THINK_INNER_RE.sub("", cleaned)
        # Remove any remaining unclosed <think> tag to end of string
        cleaned = cls._THINK_OPEN_RE.sub("", cleaned)
        return cleaned.strip()

    def _default_test_model(self) -> str:
        return "llama3.2"
