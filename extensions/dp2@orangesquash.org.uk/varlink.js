// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

import Gio from "gi://Gio";
import GLib from "gi://GLib";

export const INTERFACE_NAME = "uk.org.orangesquash.dp2";

const TEXT_ENCODER = new TextEncoder();
const TEXT_DECODER = new TextDecoder("utf-8");

/**
 * Resolves the Unix socket path for the dp2 Varlink service.
 */
export function resolveSocketPath() {
  const explicit = GLib.getenv("DP2_VARLINK_SOCKET");
  if (explicit && explicit.length > 0) return explicit;

  const runtimeDir = GLib.get_user_runtime_dir();
  return GLib.build_filenamev([runtimeDir, INTERFACE_NAME]);
}

/**
 * Pure GJS Varlink-over-Unix-socket client for uk.org.orangesquash.dp2.
 *
 * Each call opens a short-lived Unix stream connection, writes one NUL-terminated
 * JSON request, and reads one NUL-terminated JSON reply. Connecting automatically
 * triggers systemd socket activation when dp2.socket is enabled.
 */
export class Dp2VarlinkClient {
  constructor(socketPath = resolveSocketPath()) {
    this._socketPath = socketPath;
  }

  get socketPath() {
    return this._socketPath;
  }

  async call(method, parameters = {}, cancellable = null) {
    const client = new Gio.SocketClient();
    const address = Gio.UnixSocketAddress.new(this._socketPath);

    const connection = await new Promise((resolve, reject) => {
      client.connect_async(address, cancellable, (_src, res) => {
        try {
          resolve(client.connect_finish(res));
        } catch (err) {
          reject(err);
        }
      });
    });

    try {
      const payload = `${JSON.stringify({
        method: `${INTERFACE_NAME}.${method}`,
        parameters,
      })}\0`;
      const bytes = GLib.Bytes.new(TEXT_ENCODER.encode(payload));
      const outStream = connection.get_output_stream();

      await new Promise((resolve, reject) => {
        outStream.write_bytes_async(
          bytes,
          GLib.PRIORITY_DEFAULT,
          cancellable,
          (_src, res) => {
            try {
              outStream.write_bytes_finish(res);
              resolve();
            } catch (err) {
              reject(err);
            }
          },
        );
      });

      const dataIn = new Gio.DataInputStream({
        base_stream: connection.get_input_stream(),
      });

      const rawReply = await new Promise((resolve, reject) => {
        dataIn.read_upto_async(
          "\0",
          1,
          GLib.PRIORITY_DEFAULT,
          cancellable,
          (_src, res) => {
            try {
              const [line] = dataIn.read_upto_finish(res);
              resolve(line ?? "");
            } catch (err) {
              reject(err);
            }
          },
        );
      });

      if (!rawReply) throw new Error(`Empty Varlink reply for ${method}`);

      const decoded =
        typeof rawReply === "string" ? rawReply : TEXT_DECODER.decode(rawReply);
      const parsed = JSON.parse(decoded);

      if (parsed.error) {
        const detail =
          parsed.parameters?.message ?? parsed.parameters?.id ?? parsed.error;
        throw new Error(`${parsed.error}: ${detail}`);
      }

      return parsed.parameters ?? {};
    } finally {
      try {
        connection.close(null);
      } catch {
        // Connection already closed.
      }
    }
  }

  async getStatus(cancellable = null) {
    const reply = await this.call("GetStatus", {}, cancellable);
    return reply.status;
  }

  async listEffects(cancellable = null) {
    const reply = await this.call("ListEffects", {}, cancellable);
    return reply.effects ?? [];
  }

  async setEffect(effect, cancellable = null) {
    const reply = await this.call("SetEffect", { effect }, cancellable);
    return reply.effect;
  }

  async listSources(cancellable = null) {
    const reply = await this.call("ListSources", {}, cancellable);
    return reply.sources ?? [];
  }

  async setSource(name, cancellable = null) {
    return this.call("SetSource", { name }, cancellable);
  }

  async importVideo(path, select = true, cancellable = null) {
    return this.call("ImportVideo", { path, select }, cancellable);
  }

  async removeVideo(name, cancellable = null) {
    return this.call("RemoveVideo", { name }, cancellable);
  }

  async listModels(cancellable = null) {
    const reply = await this.call("ListModels", {}, cancellable);
    return reply.models ?? [];
  }

  async setModel(id, cancellable = null) {
    const reply = await this.call("SetModel", { id }, cancellable);
    return reply.model;
  }

  async getPreview(maxWidth = 260, cancellable = null) {
    const reply = await this.call(
      "GetPreview",
      { max_width: maxWidth },
      cancellable,
    );
    return reply.frame ?? null;
  }

  async resetDepthState(cancellable = null) {
    return this.call("ResetDepthState", {}, cancellable);
  }

  async shutdown(cancellable = null) {
    return this.call("Shutdown", {}, cancellable);
  }
}
