#!/usr/bin/env python3
"""Exercise a packaged uhm binary through a private-CA HTTP CONNECT proxy."""

from __future__ import annotations

import argparse
import os
import select
import socket
import socketserver
import ssl
import subprocess
import tempfile
import threading
from pathlib import Path


def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, check=True, text=True, **kwargs)


def make_certificates(root: Path) -> tuple[Path, Path, Path]:
    ca_key = root / "ca.key"
    ca_cert = root / "ca.pem"
    leaf_key = root / "leaf.key"
    leaf_csr = root / "leaf.csr"
    leaf_cert = root / "leaf.pem"
    extensions = root / "leaf.ext"
    extensions.write_text(
        "subjectAltName=DNS:api.openai.com\n"
        "keyUsage=digitalSignature,keyEncipherment\n"
        "extendedKeyUsage=serverAuth\n",
        encoding="utf-8",
    )
    quiet = {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
    run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            str(ca_key),
            "-out",
            str(ca_cert),
            "-subj",
            "/CN=uhm private test root",
            "-days",
            "1",
        ],
        **quiet,
    )
    run(
        [
            "openssl",
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            str(leaf_key),
            "-out",
            str(leaf_csr),
            "-subj",
            "/CN=api.openai.com",
        ],
        **quiet,
    )
    run(
        [
            "openssl",
            "x509",
            "-req",
            "-in",
            str(leaf_csr),
            "-CA",
            str(ca_cert),
            "-CAkey",
            str(ca_key),
            "-CAcreateserial",
            "-out",
            str(leaf_cert),
            "-days",
            "1",
            "-extfile",
            str(extensions),
        ],
        **quiet,
    )
    return ca_cert, leaf_cert, leaf_key


class TlsOrigin:
    def __init__(self, certificate: Path, key: Path) -> None:
        self.requests: list[str] = []
        self._lock = threading.Lock()
        self._socket = socket.socket()
        self._socket.bind(("127.0.0.1", 0))
        self._socket.listen()
        self.port = self._socket.getsockname()[1]
        self._context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self._context.load_cert_chain(certificate, key)
        threading.Thread(target=self._serve, daemon=True).start()

    def _serve(self) -> None:
        while True:
            try:
                client, _ = self._socket.accept()
            except OSError:
                return
            threading.Thread(target=self._handle, args=(client,), daemon=True).start()

    def _handle(self, client: socket.socket) -> None:
        try:
            stream = self._context.wrap_socket(client, server_side=True)
            data = bytearray()
            while b"\r\n\r\n" not in data and len(data) < 1_000_000:
                chunk = stream.recv(65536)
                if not chunk:
                    return
                data.extend(chunk)
            headers, _, body = bytes(data).partition(b"\r\n\r\n")
            content_length = 0
            for line in headers.split(b"\r\n")[1:]:
                name, separator, value = line.partition(b":")
                if separator and name.lower() == b"content-length":
                    content_length = int(value.strip())
            while len(body) < content_length:
                chunk = stream.recv(min(65536, content_length - len(body)))
                if not chunk:
                    break
                body += chunk
            request_line = headers.split(b"\r\n", 1)[0].decode("ascii", "replace")
            with self._lock:
                self.requests.append(request_line)
            response_body = b'{"error":{"message":"invalid diagnostic token"}}'
            stream.sendall(
                b"HTTP/1.1 401 Unauthorized\r\n"
                + f"Content-Length: {len(response_body)}\r\n".encode()
                + b"Content-Type: application/json\r\nConnection: close\r\n\r\n"
                + response_body
            )
            stream.close()
        except (OSError, ssl.SSLError, ValueError):
            client.close()


class ConnectHandler(socketserver.BaseRequestHandler):
    origin_port = 0
    destinations: list[str] = []
    lock = threading.Lock()

    def handle(self) -> None:
        data = bytearray()
        while b"\r\n\r\n" not in data and len(data) < 65536:
            chunk = self.request.recv(4096)
            if not chunk:
                return
            data.extend(chunk)
        first = bytes(data).split(b"\r\n", 1)[0].decode("ascii", "replace")
        parts = first.split()
        if len(parts) != 3 or parts[0] != "CONNECT" or parts[1] != "api.openai.com:443":
            self.request.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
            return
        with self.lock:
            self.destinations.append(parts[1])
        upstream = socket.create_connection(("127.0.0.1", self.origin_port), timeout=5)
        self.request.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        sockets = [self.request, upstream]
        while sockets:
            readable, _, _ = select.select(sockets, [], [], 5)
            if not readable:
                break
            for source in readable:
                try:
                    chunk = source.recv(65536)
                except OSError:
                    chunk = b""
                if not chunk:
                    sockets.clear()
                    break
                target = upstream if source is self.request else self.request
                target.sendall(chunk)
        upstream.close()


def clean_environment(home: Path, proxy: str) -> dict[str, str]:
    environment = os.environ.copy()
    for name in (
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "UHM_CA_BUNDLE",
    ):
        environment.pop(name, None)
    environment.update(
        {
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / "config"),
            "XDG_DATA_HOME": str(home / "data"),
            "XDG_CACHE_HOME": str(home / "cache"),
            "OPENAI_API_KEY": "invalid-diagnostic-token",
            "UHM_TELEMETRY": "off",
            "HTTPS_PROXY": proxy,
        }
    )
    return environment


def invoke(binary: Path, environment: dict[str, str], arguments: list[str]) -> str:
    result = subprocess.run(
        [str(binary), *arguments],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=20,
        check=False,
    )
    return f"exit={result.returncode}\n{result.stdout}"


def require(output: str, *needles: str) -> None:
    if not all(needle in output for needle in needles):
        raise RuntimeError(f"missing {needles!r} in product output:\n{output}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    arguments = parser.parse_args()
    binary = arguments.binary.resolve()
    if not binary.is_file():
        raise SystemExit(f"binary not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="uhm-private-ca-") as temporary:
        root = Path(temporary)
        ca_cert, leaf_cert, leaf_key = make_certificates(root)
        origin = TlsOrigin(leaf_cert, leaf_key)
        ConnectHandler.origin_port = origin.port
        proxy_server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), ConnectHandler)
        proxy_server.daemon_threads = True
        threading.Thread(target=proxy_server.serve_forever, daemon=True).start()
        proxy = f"http://127.0.0.1:{proxy_server.server_address[1]}"
        base = clean_environment(root / "home", proxy)

        unavailable = socket.socket()
        unavailable.bind(("127.0.0.1", 0))
        unavailable_port = unavailable.getsockname()[1]
        unavailable.close()
        closed_proxy = clean_environment(
            root / "closed-proxy-home", f"http://127.0.0.1:{unavailable_port}"
        )
        closed_result = invoke(binary, closed_proxy, ["doctor", "network"])
        require(closed_result, "proxy_connect", "proxy connection or CONNECT tunnel")

        malformed_proxy = clean_environment(root / "bad-proxy-home", "ftp://proxy.invalid")
        proxy_config_result = invoke(binary, malformed_proxy, ["doctor", "network"])
        require(proxy_config_result, "proxy_config", "malformed HTTPS_PROXY")

        negative = invoke(binary, base, ["doctor", "network"])
        require(negative, "tls_certificate", "UnknownIssuer")

        ssl_file = dict(base, SSL_CERT_FILE=str(ca_cert))
        ssl_file["https_proxy"] = f"http://127.0.0.1:{unavailable_port}"
        ssl_file["ALL_PROXY"] = f"http://127.0.0.1:{unavailable_port}"
        file_result = invoke(binary, ssl_file, ["doctor", "network"])
        require(file_result, "authentication", "rejected the API key")

        certificate_dir = root / "certificates"
        certificate_dir.mkdir()
        (certificate_dir / "private-root.pem").write_bytes(ca_cert.read_bytes())
        ssl_dir = dict(base, SSL_CERT_DIR=str(certificate_dir))
        dir_result = invoke(binary, ssl_dir, ["doctor", "network"])
        require(dir_result, "authentication", "rejected the API key")

        uhm_bundle = dict(base, UHM_CA_BUNDLE=str(ca_cert))
        uhm_bundle.pop("HTTPS_PROXY")
        uhm_bundle["https_proxy"] = proxy
        bundle_result = invoke(binary, uhm_bundle, ["doctor", "network"])
        require(bundle_result, "authentication", "rejected the API key")

        http_fallback = dict(base, SSL_CERT_FILE=str(ca_cert))
        http_fallback.pop("HTTPS_PROXY")
        http_fallback["HTTP_PROXY"] = proxy
        fallback_result = invoke(binary, http_fallback, ["doctor", "network"])
        require(fallback_result, "authentication", "rejected the API key")

        malformed = root / "malformed.pem"
        malformed.write_text("not a certificate\n", encoding="utf-8")
        invalid_bundle = dict(base, UHM_CA_BUNDLE=str(malformed))
        invalid_result = invoke(binary, invalid_bundle, ["doctor", "network"])
        require(invalid_result, "trust_config", "contained no certificates")

        invalid_standard = dict(base, SSL_CERT_FILE=str(malformed))
        invalid_standard_result = invoke(binary, invalid_standard, ["doctor", "network"])
        require(invalid_standard_result, "trust_config", "did not provide any valid certificates")

        invalid_json = invoke(
            binary,
            invalid_bundle,
            ["--json", "--no-telemetry", "ask", "trust diagnostic"],
        )
        require(invalid_json, '"error_kind":"trust"', "contained no certificates")

        live_result = invoke(
            binary,
            uhm_bundle,
            ["--no-telemetry", "--no-stream", "ask", "private CA regression"],
        )
        require(live_result, "HTTP 401", "invalid diagnostic token")

        proxy_server.shutdown()
        request_lines = origin.requests
        if not any(line.startswith("GET ") and "/v1/models " in line for line in request_lines):
            raise RuntimeError(f"doctor did not reach the expected route: {request_lines!r}")
        if not any(
            line.startswith("POST ") and "/v1/responses " in line for line in request_lines
        ):
            raise RuntimeError(f"live request did not reach the expected route: {request_lines!r}")
        if len(ConnectHandler.destinations) < 5:
            raise RuntimeError("expected successful CONNECT tunnels were not observed")

    print("private-CA CONNECT regression: ok")


if __name__ == "__main__":
    main()
