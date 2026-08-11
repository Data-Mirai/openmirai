#!/usr/bin/env python3
"""El runner del visualizador: convierte OpenMirai Studio de mirador en botonera.

Hasta ahora el visualizador era 100% estático — mostraba el grafo y ejecuciones
falsas embebidas en el build. Se podía LEER un agente pero no CORRERLO: para eso
había que volver a la terminal y escribir el `mirai run` a mano.

Esto levanta un servidor local mínimo (stdlib, sin dependencias) que:

  · sirve el visualizador,
  · lista los agentes reales del catálogo y de `sources.local.json`,
  · **ejecuta uno a demanda** con el binario `mirai`, y
  · devuelve el paso a paso en vivo (NDJSON por goteo) más el registro de corridas.

Todo local, en 127.0.0.1. Y como el engine ya no exige proveedor para grafos sin
IA, desde aquí se corre un workflow determinista sin tocar una sola cuota.

    python3 visualizer/servidor.py [--puerto 8790]
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import threading
import time
import webbrowser
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

RAIZ = Path(__file__).resolve().parent.parent
VIS = RAIZ / "visualizer"
REGISTRO = VIS / "ejecuciones.jsonl"          # historia real, no la demo embebida
MAX_CUERPO = 256 * 1024

# El CLI colorea con ANSI; en el navegador esos códigos salen como basura visible.
_ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def sin_ansi(t: str) -> str:
    return _ANSI.sub("", t)


def binario_mirai() -> str | None:
    for cand in (RAIZ / "target/release/mirai", RAIZ / "target/debug/mirai"):
        if cand.is_file():
            return str(cand)
    return shutil.which("mirai")


def _lee_yaml_min(texto: str) -> dict:
    """Saca name/description/inputs de un YAML de agente sin depender de PyYAML.

    Solo entiende la forma que usan los agentes de OpenMirai; con eso basta para
    pintar el formulario. Si algo no encaja, el campo queda vacío y el usuario
    escribe el JSON a mano — nunca revienta.
    """
    datos: dict = {"name": "", "description": "", "inputs": {}}
    m = re.search(r"^name:\s*(.+)$", texto, re.M)
    if m:
        datos["name"] = m.group(1).strip().strip("\"'")
    m = re.search(r"^description:\s*(.+)$", texto, re.M)
    if m:
        datos["description"] = m.group(1).strip().strip("\"'")

    bloque = re.search(r"^inputs:\s*$(.*?)^(?=\S)", texto, re.M | re.S)
    if bloque:
        actual = None
        for linea in bloque.group(1).split("\n"):
            if not linea.strip():
                continue
            campo = re.match(r"^  (\w+):\s*$", linea)
            if campo:
                actual = campo.group(1)
                datos["inputs"][actual] = {"required": False, "default": "",
                                           "description": "", "type": "text", "opciones": ""}
                continue
            if actual:
                prop = re.match(r"^    (\w+):\s*(.+)$", linea)
                if prop:
                    clave, valor = prop.group(1), prop.group(2).strip().strip("\"'")
                    if clave == "required":
                        datos["inputs"][actual]["required"] = valor == "true"
                    elif clave == "options":
                        datos["inputs"][actual]["opciones"] = " | ".join(
                            v.strip().strip('"\'') for v in
                            valor.strip("[]").split(",") if v.strip())
                    elif clave in ("default", "description", "type"):
                        datos["inputs"][actual][clave] = valor
    return datos


def catalogo() -> list[dict]:
    """Los agentes de verdad, del repo y de las fuentes locales del usuario."""
    vistos, salida = set(), []

    def suma(p: Path, fuente: str) -> None:
        try:
            if not p.is_file() or p.suffix not in (".yaml", ".yml"):
                return
            clave = str(p.resolve())
            if clave in vistos:
                return
            vistos.add(clave)
            texto = p.read_text(encoding="utf-8", errors="ignore")
            info = _lee_yaml_min(texto)
            salida.append({
                "id": info["name"] or p.stem,
                "descripcion": info["description"],
                "inputs": info["inputs"],
                "ruta": str(p),
                "fuente": fuente,
                "usa_ia": bool(re.search(r"tool_type:\s*ai/|agent/run_agent", texto)),
                "mtime": p.stat().st_mtime,
            })
        except OSError:
            return

    for carpeta, fuente in ((RAIZ / "agents", "agents"), (RAIZ / "examples", "examples")):
        if carpeta.is_dir():
            for p in sorted(carpeta.rglob("*.y*ml")):
                suma(p, fuente)

    fuentes = VIS / "sources.local.json"
    if fuentes.is_file():
        try:
            for entrada in json.loads(fuentes.read_text()):
                etiqueta = entrada.get("source", "local")
                if entrada.get("dir"):
                    d = Path(entrada["dir"])
                    if d.is_dir():
                        for p in sorted(d.rglob("*.y*ml")):
                            suma(p, etiqueta)
                if entrada.get("file"):
                    suma(Path(entrada["file"]), etiqueta)
        except (OSError, json.JSONDecodeError):
            pass

    salida.sort(key=lambda a: (a["usa_ia"], -a["mtime"]))
    return salida


def ejecuciones(limite: int = 60) -> list[dict]:
    if not REGISTRO.is_file():
        return []
    filas = []
    for linea in REGISTRO.read_text(encoding="utf-8").splitlines():
        try:
            filas.append(json.loads(linea))
        except json.JSONDecodeError:
            continue
    return filas[-limite:][::-1]


def anota(entrada: dict) -> None:
    try:
        with open(REGISTRO, "a", encoding="utf-8") as f:
            f.write(json.dumps(entrada, ensure_ascii=False) + "\n")
    except OSError:
        pass


# Carpeta donde aterrizan los archivos que el navegador sube. Vive en el disco del
# SERVIDOR a propósito: cuando Studio corra en la Rocola y el equipo entre por SSH,
# el archivo tiene que llegar a la máquina que lo va a procesar, no quedarse en la
# laptop de quien lo eligió.
SUBIDAS = VIS / "subidas"


def destino_por_defecto(ruta_audio: str) -> str:
    """Dónde escribir si el usuario no dice nada: al lado del audio, en `transcripts/`.

    Elegir carpeta a mano es el paso que más fricción metía y el que menos decisión
    real tiene: el 90% de las veces uno quiere el resultado junto al original.
    """
    try:
        p = Path(ruta_audio)
        base = p.parent if p.parent.name != SUBIDAS.name else Path.home() / "Documents"
        return str(base / "transcripts")
    except Exception:                                     # noqa: BLE001
        return str(Path.home() / "Documents" / "transcripts")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, formato, *args):
        pass

    def _envia(self, codigo: int, cuerpo: bytes, tipo: str):
        self.send_response(codigo)
        self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(cuerpo)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(cuerpo)

    def _json(self, codigo: int, datos: dict):
        self._envia(codigo, json.dumps(datos, ensure_ascii=False).encode(),
                    "application/json; charset=utf-8")

    def do_GET(self):
        ruta = urlparse(self.path).path
        if ruta in ("/", "/index.html", "/studio"):
            p = VIS / "estudio.html"
            if not p.is_file():
                p = VIS / "mirai-app.html"
            return self._envia(200, p.read_bytes(), "text/html; charset=utf-8")
        if ruta == "/api/agentes":
            return self._json(200, {"agentes": catalogo(), "mirai": bool(binario_mirai())})
        if ruta == "/api/sugerir-salida":
            q = parse_qs(urlparse(self.path).query)
            return self._json(200, {"ruta": destino_por_defecto(q.get("audio", [""])[0])})
        if ruta == "/api/ejecuciones":
            return self._json(200, {"ejecuciones": ejecuciones()})
        if ruta == "/api/agente":
            q = parse_qs(urlparse(self.path).query)
            p = Path(q.get("ruta", [""])[0])
            if not p.is_file() or p.suffix not in (".yaml", ".yml"):
                return self._json(404, {"error": "no existe"})
            return self._envia(200, p.read_bytes(), "text/plain; charset=utf-8")
        return self._json(404, {"error": "no existe"})

    def do_POST(self):
        try:
            ruta_post = urlparse(self.path).path
            if ruta_post == "/api/subir":
                # El navegador manda el archivo tal cual en el cuerpo; el nombre viaja
                # en una cabecera. Sin multipart: una dependencia menos y el streaming
                # a disco es directo, que importa con audios de cientos de megas.
                nombre = self.headers.get("X-Nombre") or "audio"
                nombre = re.sub(r"[^A-Za-z0-9._-]", "_", Path(nombre).name)[:120] or "audio"
                largo = int(self.headers.get("Content-Length") or 0)
                if largo <= 0:
                    return self._json(400, {"error": "archivo vacío"})
                SUBIDAS.mkdir(parents=True, exist_ok=True)
                destino = SUBIDAS / nombre
                escrito = 0
                with open(destino, "wb") as f:
                    while escrito < largo:
                        trozo = self.rfile.read(min(1 << 20, largo - escrito))
                        if not trozo:
                            break
                        f.write(trozo)
                        escrito += len(trozo)
                if escrito < largo:
                    destino.unlink(missing_ok=True)
                    return self._json(400, {"error": "la subida se cortó"})
                return self._json(200, {"ruta": str(destino), "bytes": escrito,
                                        "sugerencia_salida": destino_por_defecto(str(destino))})
            if ruta_post != "/api/ejecutar":
                return self._json(404, {"error": "no existe"})
            largo = int(self.headers.get("Content-Length") or 0)
            if largo <= 0 or largo > MAX_CUERPO:
                return self._json(400, {"error": "cuerpo inválido"})
            datos = json.loads(self.rfile.read(largo) or b"{}")
            if not isinstance(datos, dict):
                return self._json(400, {"error": "se esperaba un objeto"})

            ruta = Path(str(datos.get("ruta", "")))
            if not ruta.is_file() or ruta.suffix not in (".yaml", ".yml"):
                return self._json(422, {"error": f"no existe el agente: {ruta}"})
            entradas = datos.get("inputs") or {}
            if not isinstance(entradas, dict):
                return self._json(422, {"error": "inputs debe ser un objeto"})

            mirai = binario_mirai()
            if not mirai:
                return self._json(503, {"error": "no encontré el binario `mirai`. "
                                                 "Compílalo: cargo build --release"})

            cmd = [mirai, "run", str(ruta), "--input", json.dumps(entradas, ensure_ascii=False)]
            for extra in ("provider", "model"):
                if datos.get(extra):
                    cmd += [f"--{extra}", str(datos[extra])]

            self.send_response(200)
            self.send_header("Content-Type", "application/x-ndjson; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Connection", "close")
            self.end_headers()

            t0 = time.time()
            proc = subprocess.Popen(cmd, cwd=str(RAIZ), stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, text=True, bufsize=1)
            crudo, en_json = [], False
            try:
                for linea in proc.stdout:
                    crudo.append(linea)
                    # El CLI mezcla el progreso legible con el JSON final del run. El
                    # JSON completo en la consola es ruido: lo que el usuario quiere ver
                    # es el paso a paso. Se corta al primer '{' de la respuesta y el
                    # resultado se resume abajo, en el evento de cierre.
                    if not en_json and linea.startswith("{"):
                        en_json = True
                        self.wfile.write((json.dumps({"linea": "· recogiendo el resultado…"},
                                                     ensure_ascii=False) + "\n").encode())
                        self.wfile.flush()
                        continue
                    if en_json:
                        continue
                    self.wfile.write((json.dumps({"linea": sin_ansi(linea.rstrip())},
                                                 ensure_ascii=False) + "\n").encode())
                    self.wfile.flush()
                proc.wait(timeout=15)
            except (BrokenPipeError, OSError):
                proc.kill()
                return None
            except subprocess.TimeoutExpired:
                proc.kill()

            texto = "".join(crudo)
            i = texto.find("{")
            resultado, estado = None, "error"
            if i >= 0:
                try:
                    resultado = json.loads(texto[i:])
                    estado = str(resultado.get("status", "")).lower() or "error"
                except json.JSONDecodeError:
                    pass
            segs = round(time.time() - t0, 1)
            resumen = ""
            if resultado:
                st = resultado.get("state") or {}
                # El nodo output/response deja su valor en `result`. Ojo: `state` llega
                # ORDENADO ALFABÉTICAMENTE, no por orden de ejecución — recorrerlo al azar
                # tomaba el stdout de `verificar` ("audio OK…") y el historial mostraba el
                # chequeo previo como si fuera el resultado del run.
                for v in st.values():
                    if isinstance(v, dict) and v.get("result"):
                        resumen = str(v["result"])[:400]
                        break
                if not resumen:
                    # sin nodo de salida: el último paso del grafo que dejó stdout
                    orden = [n.get("id") for n in (resultado.get("graph") or {}).get("nodes", [])]
                    for nodo in reversed(orden or list(st)):
                        v = st.get(nodo)
                        if isinstance(v, dict) and v.get("stdout"):
                            resumen = str(v["stdout"])[:400]
                            break
            resumen = sin_ansi(resumen).strip()
            registro = {"ts": datetime.now().isoformat(timespec="seconds"),
                        "agente": ruta.stem, "ruta": str(ruta), "estado": estado,
                        "segundos": segs, "inputs": entradas, "resumen": resumen}
            anota(registro)
            try:
                self.wfile.write((json.dumps({"fin": registro}, ensure_ascii=False) + "\n").encode())
                self.wfile.flush()
            except OSError:
                pass
            return None
        except Exception as ex:                       # noqa: BLE001 — el hilo no muere
            try:
                return self._json(500, {"error": f"no procesable: {type(ex).__name__}"})
            except OSError:
                return None


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--puerto", type=int, default=8790)
    ap.add_argument("--no-abrir", action="store_true")
    a = ap.parse_args()

    for intento in range(10):
        try:
            httpd = ThreadingHTTPServer(("127.0.0.1", a.puerto + intento), Handler)
            break
        except OSError:
            continue
    else:
        raise SystemExit("no encontré puerto libre")

    url = f"http://127.0.0.1:{httpd.server_address[1]}"
    n = len(catalogo())
    print(f"OpenMirai Studio en {url}")
    print(f"  {n} agentes · binario mirai: {'sí' if binario_mirai() else 'NO (cargo build --release)'}")
    print("  solo tu máquina · Ctrl+C para parar")
    if not a.no_abrir:
        threading.Timer(0.6, lambda: webbrowser.open(url)).start()
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\nStudio detenido.")
    finally:
        httpd.server_close()


if __name__ == "__main__":
    main()
