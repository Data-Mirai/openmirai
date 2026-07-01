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
        relpath: path.relative(REPO, p),
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

if (!fs.existsSync(JSYAML)) { console.error('ERROR: falta ' + JSYAML + ' (copia js-yaml.min.js ahi).'); process.exit(1); }
const jsyaml = fs.readFileSync(JSYAML, 'utf8');

let html = fs.readFileSync(HTML, 'utf8');
const re = /<!-- @BUILD_INJECT@[\s\S]*?<script>\n'use strict';/;
if (!re.test(html)) { console.error('ERROR: marcador @BUILD_INJECT@ no encontrado en mirai-app.html'); process.exit(1); }

const inject = '<!-- @BUILD_INJECT@ (generado por build-catalog.js — no editar a mano) -->\n'
  + '<script>' + jsyaml + '</script>\n'
  + '<script>window.MIRAI_DEFAULT_SOURCES=' + JSON.stringify(sources) + ';</script>\n'
  + "<script>\n'use strict';";

html = html.replace(re, inject);
fs.writeFileSync(HTML, html);
console.log('Catalogo embebido (' + sources.length + '): ' + sources.map(s => s.source + '/' + s.id).join(', '));
