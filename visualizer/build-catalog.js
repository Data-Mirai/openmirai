#!/usr/bin/env node
/* OpenMirai Studio — build-catalog.js
 * Embebe el parser js-yaml + el YAML crudo de los agentes (agents/ + examples/)
 * dentro de mirai-app.html, para que el visualizador funcione 100% offline (doble-click).
 * Reejecutar tras clonar el repo o crear/editar agentes:  node visualizer/build-catalog.js
 */
const fs = require('fs');
const path = require('path');

const VIS = __dirname;                       // .../Engine/visualizer
const REPO = path.dirname(VIS);              // .../Engine (raiz del repo OpenMirai)
const HTML = path.join(VIS, 'mirai-app.html');
const JSYAML = path.join(VIS, 'js-yaml.min.js');

function collect(dir, source) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir)
    .filter(f => /\.(ya?ml)$/i.test(f))
    .sort()
    .map(f => {
      const p = path.join(dir, f);
      const st = fs.statSync(p);
      return {
        id: f.replace(/\.(ya?ml)$/i, ''),
        path: p,
        relpath: (function(){ var r = path.relative(REPO, p); return r.indexOf('..') === 0 ? p : r; })(),
        source,
        mtimeMin: Math.round((Date.now() - st.mtimeMs) / 60000),
        yaml: fs.readFileSync(p, 'utf8')
      };
    });
}

const sources = [].concat(
  collect(path.join(REPO, 'agents'), 'agents'),
  collect(path.join(REPO, 'examples'), 'examples')
);

// Fuentes locales extra (agentes fuera del repo), git-ignoradas — NO se comparten en el repo.
// visualizer/sources.local.json = [ { "dir": "/ruta/a/agents", "source": "etiqueta" }, { "file": "/ruta/agente.yaml", "source": "etiqueta" } ]
try {
  const localCfg = path.join(VIS, 'sources.local.json');
  if (fs.existsSync(localCfg)) {
    const extra = JSON.parse(fs.readFileSync(localCfg, 'utf8'));
    (Array.isArray(extra) ? extra : []).forEach(function (e) {
      if (e && e.dir) {
        sources.push.apply(sources, collect(e.dir, e.source || 'externo'));
      } else if (e && e.file && fs.existsSync(e.file)) {
        const st = fs.statSync(e.file);
        sources.push({ id: path.basename(e.file).replace(/\.(ya?ml)$/i, ''), path: e.file, relpath: e.file, source: e.source || 'externo', mtimeMin: Math.round((Date.now() - st.mtimeMs) / 60000), yaml: fs.readFileSync(e.file, 'utf8') });
      }
    });
  }
} catch (e) { console.error('AVISO: sources.local.json invalido, se ignora:', e.message); }

if (!fs.existsSync(JSYAML)) { console.error('ERROR: falta ' + JSYAML + ' (copia js-yaml.min.js ahi).'); process.exit(1); }
const jsyaml = fs.readFileSync(JSYAML, 'utf8');

// Historial de ejecuciones REALES por id de agente (git-ignorado, preferencia local).
// visualizer/runs.local.json = { "<agent-id>": [ { "status":"success|error", "tMinAgo":N, "durationSec":N, "summary":"..." } ] }
// Se generan corriendo agentes de verdad contra el engine (Ollama/…); pueblan el dashboard (Inicio + Detalle).
let runsByAgent = {};
try {
  const runsCfg = path.join(VIS, 'runs.local.json');
  if (fs.existsSync(runsCfg)) runsByAgent = JSON.parse(fs.readFileSync(runsCfg, 'utf8')) || {};
} catch (e) { console.error('AVISO: runs.local.json invalido, se ignora:', e.message); }

let html = fs.readFileSync(HTML, 'utf8');
const re = /<!-- @BUILD_INJECT@[\s\S]*?<script>\n'use strict';/;
if (!re.test(html)) { console.error('ERROR: marcador @BUILD_INJECT@ no encontrado en mirai-app.html'); process.exit(1); }

const inject = '<!-- @BUILD_INJECT@ (generado por build-catalog.js — no editar a mano) -->\n'
  + '<script>' + jsyaml + '</script>\n'
  + '<script>window.MIRAI_DEFAULT_SOURCES=' + JSON.stringify(sources) + ';</script>\n'
  + '<script>window.MIRAI_RUNS=' + JSON.stringify(runsByAgent) + ';</script>\n'
  + "<script>\n'use strict';";

html = html.replace(re, inject);
fs.writeFileSync(HTML, html);
console.log('Catalogo embebido (' + sources.length + '): ' + sources.map(s => s.source + '/' + s.id).join(', '));
