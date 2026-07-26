#!/usr/bin/env node
/* claude-code-proxy.js — Shim OpenAI-compatible sobre la SUSCRIPCION de Claude Code.
 *
 * POR QUE existe: el adapter `claude` del engine (cli/src/adapter_factory.rs) SOLO
 * habla la API REST metered de Anthropic (api.anthropic.com + ANTHROPIC_API_KEY).
 * La suscripcion de Claude Code NO expone esa API. Pero el CLI `claude -p` SI
 * responde headless usando la suscripcion (sin API key). Este proxy traduce:
 *
 *   engine  --(POST /chat/completions, formato OpenAI)-->  este proxy
 *   proxy   --(shell-out `claude -p --output-format json`)-->  suscripcion
 *   proxy   --(choices[0].message.content)-->  engine
 *
 * Arrancar el engine apuntando aca:
 *   mirai serve --provider openai --base-url http://127.0.0.1:8787/v1  (o env MIRAI_LLM_*)
 *
 * Zero dependencias (solo Node stdlib). No usa API key. No toca .env.
 */
'use strict';
const http = require('http');
const { spawn } = require('child_process');

const PORT = parseInt(process.env.CLAUDE_PROXY_PORT || '8787', 10);
const CLAUDE_BIN = process.env.CLAUDE_BIN || 'claude';
// Modelo por defecto del CLI = el de la suscripcion; se puede forzar con CLAUDE_PROXY_MODEL.
const FORCED_MODEL = process.env.CLAUDE_PROXY_MODEL || '';

function log(...a) { console.log(new Date().toISOString(), ...a); }

// Ejecuta `claude -p` con el prompt por stdin, devuelve {text, meta}.
function callClaude(prompt, model) {
  return new Promise((resolve, reject) => {
    // --max-turns 1: caps agentic loops. Sin esto, ciertas queries hacen que
    // `claude -p` entre en modo agente y genere miles de tokens (visto: 1 nodo
    // colgado 112s / 7208 tokens). Para "nodo LLM = una respuesta directa" 1 turno basta.
    // --system-prompt: fuerza respuesta directa, sin tools ni preambulo.
    const SYS = 'You are a direct LLM inference engine embedded in an agent graph. Answer the user prompt directly and concisely, then stop. Do not use tools, do not ask questions, do not add preamble.';
    const args = ['-p', '--output-format', 'json', '--max-turns', '1', '--system-prompt', SYS];
    // El CLI de la suscripcion solo acepta aliases cortos (sonnet/opus/haiku),
    // NO ids largos tipo "claude-sonnet-4-*" (retirados) ni "claude-sonnet".
    // Mapeamos a un alias seguro; si no reconocemos el modelo, NO pasamos --model
    // y dejamos que el CLI use el default de la suscripcion.
    let alias = FORCED_MODEL;
    if (!alias && model) {
      const lm = String(model).toLowerCase();
      if (lm.includes('opus')) alias = 'opus';
      else if (lm.includes('haiku')) alias = 'haiku';
      else if (lm.includes('sonnet')) alias = 'sonnet';
    }
    if (alias) args.push('--model', alias);
    const child = spawn(CLAUDE_BIN, args, { stdio: ['pipe', 'pipe', 'pipe'] });
    let out = '', err = '';
    child.stdout.on('data', d => out += d);
    child.stderr.on('data', d => err += d);
    child.on('error', reject);
    child.on('close', code => {
      if (code !== 0) return reject(new Error(`claude exited ${code}: ${err.slice(0, 500)}`));
      try {
        const j = JSON.parse(out);
        if (j.is_error) return reject(new Error(`claude error: ${j.result || j.subtype}`));
        resolve({
          text: (j.result != null ? String(j.result) : ''),
          in: (j.usage && j.usage.input_tokens) || 0,
          out: (j.usage && j.usage.output_tokens) || 0,
        });
      } catch (e) {
        // Fallback: si no vino JSON, usar el texto crudo.
        if (out.trim()) return resolve({ text: out.trim(), in: 0, out: 0 });
        reject(new Error(`parse fail: ${e.message}; raw: ${out.slice(0, 300)}`));
      }
    });
    child.stdin.write(prompt);
    child.stdin.end();
  });
}

// Aplana los messages OpenAI en UN prompt para claude -p.
function flatten(messages) {
  const parts = [];
  for (const m of (messages || [])) {
    let c = m.content;
    if (Array.isArray(c)) c = c.map(p => (p && p.text) ? p.text : (typeof p === 'string' ? p : '')).join('\n');
    c = (c == null ? '' : String(c)).trim();
    if (!c) continue;
    if (m.role === 'system') parts.push(`[SYSTEM]\n${c}`);
    else if (m.role === 'assistant') parts.push(`[ASSISTANT]\n${c}`);
    else parts.push(c); // user
  }
  return parts.join('\n\n').trim() || 'Hello';
}

const server = http.createServer((req, res) => {
  const send = (code, obj) => {
    const body = JSON.stringify(obj);
    res.writeHead(code, { 'content-type': 'application/json' });
    res.end(body);
  };

  if (req.method === 'GET' && req.url.replace(/\/$/, '').endsWith('/models')) {
    return send(200, { object: 'list', data: [{ id: 'claude-code-subscription', object: 'model', owned_by: 'anthropic' }] });
  }
  if (req.method === 'GET' && (req.url === '/health' || req.url === '/')) {
    return send(200, { status: 'ok', proxy: 'claude-code-subscription' });
  }
  if (req.method !== 'POST' || !/\/chat\/completions\/?$/.test(req.url)) {
    return send(404, { error: { message: `no route for ${req.method} ${req.url}` } });
  }

  let raw = '';
  req.on('data', d => raw += d);
  req.on('end', async () => {
    let payload = {};
    try { payload = JSON.parse(raw || '{}'); } catch (e) { return send(400, { error: { message: 'bad json' } }); }
    const prompt = flatten(payload.messages);
    const model = payload.model || '';
    const t0 = Date.now();
    try {
      const r = await callClaude(prompt, model);
      log(`OK ${Date.now() - t0}ms in=${r.in} out=${r.out} model=${model || '(sub-default)'} chars=${r.text.length}`);
      send(200, {
        id: 'chatcmpl-cc-' + Date.now(),
        object: 'chat.completion',
        created: Math.floor(Date.now() / 1000),
        model: model || 'claude-code-subscription',
        choices: [{ index: 0, message: { role: 'assistant', content: r.text }, finish_reason: 'stop' }],
        usage: { prompt_tokens: r.in, completion_tokens: r.out, total_tokens: r.in + r.out },
      });
    } catch (e) {
      log('ERR', e.message);
      send(502, { error: { message: e.message, type: 'claude_cli_error' } });
    }
  });
});

server.listen(PORT, '127.0.0.1', () => log(`claude-code-proxy listening on http://127.0.0.1:${PORT}  (/v1/chat/completions)`));
