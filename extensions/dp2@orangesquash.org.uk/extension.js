// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

import Clutter from "gi://Clutter";
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import GObject from "gi://GObject";
import St from "gi://St";

import { Extension } from "resource:///org/gnome/shell/extensions/extension.js";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
import * as PanelMenu from "resource:///org/gnome/shell/ui/panelMenu.js";
import * as PopupMenu from "resource:///org/gnome/shell/ui/popupMenu.js";

import { PreviewMenuItem } from "./preview.js";
import { Dp2VarlinkClient } from "./varlink.js";

const PREVIEW_INTERVAL_MS = 250;

const COLORMAPS = [
  { id: "turbo", label: "Turbo" },
  { id: "viridis", label: "Viridis" },
  { id: "magma", label: "Magma" },
  { id: "grayscale", label: "Grey" },
];

const EFFECT_ICONS = {
  "depth-map": "preferences-color-symbolic",
  "depth-of-field": "camera-photo-symbolic",
  "background-blur": "view-reveal-symbolic",
  "background-replace": "image-x-generic-symbolic",
  "split-view": "view-dual-symbolic",
  passthrough: "camera-web-symbolic",
};

function shortModelLabel(model) {
  const id = (model.id ?? "").toLowerCase();
  const tokens = id.split(/[-_]/);
  const isStream = tokens.some((t) => t.startsWith("stream"));
  let tier = model.id.toUpperCase();
  if (tokens.includes("s")) tier = "S";
  else if (tokens.includes("b")) tier = "B";
  else if (tokens.includes("l")) tier = "L";
  else if (tokens.includes("h")) tier = "H+";
  return isStream ? `${tier} Stream` : tier;
}

function modelIconName(model) {
  const id = (model.id ?? "").toLowerCase();
  const tokens = id.split(/[-_]/);
  if (tokens.some((t) => t.startsWith("stream")))
    return "dp2-model-stream-symbolic";
  if (tokens.includes("b")) return "dp2-model-b-symbolic";
  if (tokens.includes("l") || tokens.includes("h"))
    return "dp2-model-l-symbolic";
  return "dp2-model-s-symbolic";
}

function formatLatency(ms) {
  if (typeof ms !== "number" || !Number.isFinite(ms)) return null;
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.round(ms)} ms`;
}

function cleanSourceLabel(description, name) {
  const raw = (description || name || "").trim();
  return raw.replace(/^(?:PipeWire\s+source|Video:)\s+/i, "");
}

const Dp2Indicator = GObject.registerClass(
  class Dp2Indicator extends PanelMenu.Button {
    _init(extensionPath) {
      super._init(0.5, "Depth Pro 2 Camera", false);

      this._extensionPath = extensionPath;
      this._client = new Dp2VarlinkClient();
      this._previewTimerId = 0;
      this._refreshInFlight = false;
      this._lastStatus = null;
      this._lastPreviewSeq = -1;

      const panelBox = new St.BoxLayout({
        style_class: "panel-status-menu-box dp2-panel-box",
      });
      this._panelIcon = new St.Icon({
        icon_name: "camera-video-symbolic",
        style_class: "system-status-icon",
      });
      panelBox.add_child(this._panelIcon);
      this.add_child(panelBox);

      this._buildMenu();

      this.menu.connect("open-state-changed", (_menu, isOpen) => {
        if (isOpen) {
          this.refresh();
          this._startPreviewPolling();
        } else {
          this._stopPreviewPolling();
        }
      });
    }

    _customIcon(iconName) {
      const path = GLib.build_filenamev([
        this._extensionPath,
        "icons",
        `${iconName}.svg`,
      ]);
      return Gio.icon_new_for_string(path);
    }

    _buildMenu() {
      this.menu.box.add_style_class_name("dp2-menu");

      this._previewItem = new PreviewMenuItem();
      this.menu.addMenuItem(this._previewItem);

      this._modelSection = new PopupMenu.PopupMenuSection();
      this.menu.addMenuItem(this._modelSection);

      this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem("Effect"));
      this._effectsSection = new PopupMenu.PopupMenuSection();
      this.menu.addMenuItem(this._effectsSection);

      this._optionsSection = new PopupMenu.PopupMenuSection();
      this.menu.addMenuItem(this._optionsSection);

      this.menu.addMenuItem(
        new PopupMenu.PopupSeparatorMenuItem("Input Source"),
      );
      this._sourcesSection = new PopupMenu.PopupMenuSection();
      this.menu.addMenuItem(this._sourcesSection);

      this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
      this._buildFooter();
    }

    _buildFooter() {
      const footerItem = new PopupMenu.PopupBaseMenuItem({
        reactive: false,
        can_focus: false,
        style_class: "dp2-footer-item",
      });

      const row = new St.BoxLayout({
        x_expand: true,
        style_class: "dp2-footer-row",
      });

      const resetBtn = this._makeActionButton(
        "view-refresh-symbolic",
        "Reset Depth",
      );
      resetBtn.connect("clicked", () => {
        this._client
          .resetDepthState()
          .catch((err) => logError(err, "dp2 reset depth failed"));
      });

      const stopBtn = this._makeActionButton(
        "media-playback-stop-symbolic",
        "Stop",
      );
      stopBtn.add_style_class_name("dp2-footer-stop");
      stopBtn.connect("clicked", () => {
        this._client
          .shutdown()
          .then(() => {
            this._previewItem.showPlaceholder("Service stopped");
            this._previewItem.setCaption("Open menu to reconnect");
          })
          .catch((err) => logError(err, "dp2 shutdown failed"));
      });

      row.add_child(resetBtn);
      row.add_child(stopBtn);
      footerItem.add_child(row);
      this.menu.addMenuItem(footerItem);
    }

    _makeActionButton(iconName, labelText) {
      const content = new St.BoxLayout({
        x_align: Clutter.ActorAlign.CENTER,
        y_align: Clutter.ActorAlign.CENTER,
        style_class: "dp2-button-content",
      });
      content.add_child(
        new St.Icon({
          icon_name: iconName,
          icon_size: 14,
        }),
      );
      content.add_child(
        new St.Label({
          text: labelText,
          y_align: Clutter.ActorAlign.CENTER,
        }),
      );
      return new St.Button({
        child: content,
        can_focus: true,
        x_expand: true,
        style_class: "dp2-footer-button",
      });
    }

    async refresh() {
      if (this._refreshInFlight) return;
      this._refreshInFlight = true;

      try {
        const [status, effects, sources, models] = await Promise.all([
          this._client.getStatus(),
          this._client.listEffects(),
          this._client.listSources(),
          this._client.listModels(),
        ]);

        this._lastStatus = status;
        this._updateCaption(status);
        this._rebuildModels(models, status.active_model);
        this._rebuildEffects(effects, status.active_effect);
        this._rebuildOptions(status.active_effect);
        this._rebuildSources(sources, status.source_name);
        await this._pollPreviewOnce();
      } catch (err) {
        this._previewItem.showPlaceholder("dp2 service unavailable");
        this._previewItem.setCaption(String(err.message ?? err));
      } finally {
        this._refreshInFlight = false;
      }
    }

    _updateCaption(status) {
      const parts = [];
      if (status.frame_dimensions)
        parts.push(
          `${status.frame_dimensions.width}×${status.frame_dimensions.height}`,
        );
      const latency = formatLatency(status.last_inference_ms);
      if (latency) parts.push(latency);
      if (status.source_name && status.source_name !== "none")
        parts.push(
          cleanSourceLabel(status.source_description, status.source_name),
        );

      this._previewItem.setCaption(
        parts.length > 0
          ? parts.join(" · ")
          : "Ready — choose a camera or video",
      );
    }

    _rebuildModels(models, activeModelId) {
      this._modelSection.removeAll();
      if (models.length === 0) return;

      const rowItem = new PopupMenu.PopupBaseMenuItem({
        reactive: false,
        can_focus: false,
        style_class: "dp2-segmented-item",
      });

      const box = new St.BoxLayout({
        orientation: Clutter.Orientation.VERTICAL,
        x_expand: true,
        style_class: "dp2-segmented-box",
      });

      const linked = new St.BoxLayout({
        x_expand: true,
        style_class: "dp2-linked",
      });

      models.forEach((model, index) => {
        const isActive = model.id === activeModelId;
        const content = new St.BoxLayout({
          x_align: Clutter.ActorAlign.CENTER,
          y_align: Clutter.ActorAlign.CENTER,
          style_class: "dp2-button-content",
        });
        content.add_child(
          new St.Icon({
            gicon: this._customIcon(modelIconName(model)),
            icon_size: 14,
          }),
        );
        content.add_child(
          new St.Label({
            text: shortModelLabel(model),
            y_align: Clutter.ActorAlign.CENTER,
          }),
        );

        const classes = ["dp2-linked-button"];
        if (index === 0) classes.push("dp2-linked-first");
        if (index === models.length - 1) classes.push("dp2-linked-last");
        if (isActive) classes.push("dp2-linked-active");

        const btn = new St.Button({
          child: content,
          can_focus: model.available,
          reactive: model.available,
          x_expand: true,
          style_class: classes.join(" "),
        });
        if (model.available) {
          btn.connect("clicked", () => {
            this.selectModel(model.id).catch((err) =>
              logError(err, "dp2 setModel failed"),
            );
          });
        }
        linked.add_child(btn);
      });

      box.add_child(linked);
      rowItem.add_child(box);
      this._modelSection.addMenuItem(rowItem);
    }

    _rebuildEffects(effects, activeEffect) {
      this._effectsSection.removeAll();

      for (const effect of effects) {
        const iconName =
          EFFECT_ICONS[effect.id] ?? "applications-graphics-symbolic";
        const item = new PopupMenu.PopupImageMenuItem(effect.name, iconName);
        const isActive = activeEffect && activeEffect.id === effect.id;
        item.setOrnament(
          isActive ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE,
        );
        item.connect("activate", () => {
          this.selectEffect(effect.id).catch((err) =>
            logError(err, "dp2 setEffect failed"),
          );
        });
        this._effectsSection.addMenuItem(item);
      }
    }

    _rebuildOptions(activeEffect) {
      this._optionsSection.removeAll();
      if (!activeEffect) return;

      if (activeEffect.id === "depth-map") {
        this._addLinkedRow(
          COLORMAPS,
          activeEffect.colormap ?? "turbo",
          (colormapId) =>
            this._applyEffect({
              ...activeEffect,
              colormap: colormapId,
            }),
        );

        const invertToggle = new PopupMenu.PopupSwitchMenuItem(
          "Invert Depth",
          Boolean(activeEffect.invert),
        );
        invertToggle.connect("toggled", (_item, state) => {
          this._applyEffect({
            ...activeEffect,
            invert: state,
          }).catch((err) => logError(err, "dp2 invert toggle failed"));
        });
        this._optionsSection.addMenuItem(invertToggle);
        return;
      }

      if (activeEffect.id === "depth-of-field") {
        const params = activeEffect.parameters ?? {};
        this._addLinkedRow(
          [
            { id: "4", label: "Soft" },
            { id: "8", label: "Medium" },
            { id: "14", label: "Strong" },
          ],
          params.blur_radius ?? "8",
          (radius) =>
            this._applyCustomParams(activeEffect, { blur_radius: radius }),
        );
        this._addLinkedRow(
          [
            { id: "auto", label: "Auto" },
            { id: "1.0", label: "Near" },
            { id: "2.5", label: "Mid" },
            { id: "5.0", label: "Far" },
          ],
          params.focus_meters ?? "auto",
          (focus) =>
            this._applyCustomParams(activeEffect, { focus_meters: focus }),
        );
        return;
      }

      if (activeEffect.id === "background-blur") {
        const params = activeEffect.parameters ?? {};
        this._addLinkedRow(
          [
            { id: "1.5", label: "1.5m" },
            { id: "2.5", label: "2.5m" },
            { id: "4.0", label: "4.0m" },
          ],
          params.threshold_meters ?? "2.5",
          (threshold) =>
            this._applyCustomParams(activeEffect, {
              threshold_meters: threshold,
            }),
        );
        return;
      }

      if (activeEffect.id === "background-replace") {
        const params = activeEffect.parameters ?? {};
        this._addLinkedRow(
          [
            { id: "gradient", label: "Sunset" },
            { id: "#1b3a57", label: "Studio" },
            { id: "#1f2428", label: "Dark" },
            { id: "#00b140", label: "Chroma" },
          ],
          params.background ?? "gradient",
          (bg) => this._applyCustomParams(activeEffect, { background: bg }),
        );
      }
    }

    _addLinkedRow(options, activeId, onSelect) {
      const rowItem = new PopupMenu.PopupBaseMenuItem({
        reactive: false,
        can_focus: false,
        style_class: "dp2-segmented-item",
      });

      const linked = new St.BoxLayout({
        x_expand: true,
        style_class: "dp2-linked",
      });

      options.forEach((opt, index) => {
        const classes = ["dp2-linked-button"];
        if (index === 0) classes.push("dp2-linked-first");
        if (index === options.length - 1) classes.push("dp2-linked-last");
        if (opt.id === activeId) classes.push("dp2-linked-active");

        const btn = new St.Button({
          label: opt.label,
          can_focus: true,
          x_expand: true,
          style_class: classes.join(" "),
        });
        btn.connect("clicked", () => {
          onSelect(opt.id).catch((err) =>
            logError(err, `dp2 option ${opt.id} failed`),
          );
        });
        linked.add_child(btn);
      });

      rowItem.add_child(linked);
      this._optionsSection.addMenuItem(rowItem);
    }

    _rebuildSources(sources, activeSourceName) {
      this._sourcesSection.removeAll();

      for (const src of sources) {
        const iconName = src.is_imported_video
          ? "video-x-generic-symbolic"
          : "camera-web-symbolic";
        const label = cleanSourceLabel(src.description, src.name);
        const item = new PopupMenu.PopupImageMenuItem(label, iconName);
        item.setOrnament(
          src.name === activeSourceName
            ? PopupMenu.Ornament.CHECK
            : PopupMenu.Ornament.NONE,
        );
        item.connect("activate", () => {
          this.selectSource(src.name).catch((err) =>
            logError(err, "dp2 setSource failed"),
          );
        });
        this._sourcesSection.addMenuItem(item);
      }

      const chooseVideo = new PopupMenu.PopupImageMenuItem(
        "Choose Video…",
        "document-open-symbolic",
      );
      chooseVideo.connect("activate", () => {
        this._chooseVideoFile().catch((err) =>
          logError(err, "dp2 choose video failed"),
        );
      });
      this._sourcesSection.addMenuItem(chooseVideo);
    }

    async _applyEffect(effectSettings) {
      await this._client.setEffect(effectSettings);
      await this.refresh();
    }

    async _applyCustomParams(activeEffect, patch) {
      const parameters = { ...(activeEffect.parameters ?? {}), ...patch };
      await this._applyEffect({
        id: activeEffect.id,
        parameters,
      });
    }

    async selectEffect(id, extra = {}) {
      await this._applyEffect({ id, ...extra });
    }

    async selectSource(name) {
      await this._client.setSource(name);
      await this.refresh();
    }

    async selectModel(id) {
      await this._client.setModel(id);
      await this.refresh();
    }

    async importVideoPath(path) {
      await this._client.importVideo(path, true);
      await this.refresh();
    }

    async _chooseVideoFile() {
      const proc = Gio.Subprocess.new(
        [
          "zenity",
          "--file-selection",
          "--title=Choose Looping Video for Depth Pro 2",
          "--file-filter=Video files | *.mp4 *.webm *.mkv *.mov *.avi *.m4v",
        ],
        Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE,
      );

      const stdout = await new Promise((resolve, reject) => {
        proc.communicate_utf8_async(null, null, (sub, res) => {
          try {
            const [, out] = sub.communicate_utf8_finish(res);
            resolve(sub.get_successful() ? (out ?? "").trim() : "");
          } catch (err) {
            reject(err);
          }
        });
      });

      if (stdout.length > 0) await this.importVideoPath(stdout);
    }

    _startPreviewPolling() {
      this._stopPreviewPolling();
      this._previewTimerId = GLib.timeout_add(
        GLib.PRIORITY_DEFAULT,
        PREVIEW_INTERVAL_MS,
        () => {
          this._pollPreviewOnce().catch(() => {});
          return GLib.SOURCE_CONTINUE;
        },
      );
    }

    _stopPreviewPolling() {
      if (this._previewTimerId !== 0) {
        GLib.source_remove(this._previewTimerId);
        this._previewTimerId = 0;
      }
    }

    async _pollPreviewOnce() {
      const frame = await this._client.getPreview(288);
      if (!frame) return;

      if (frame.sequence !== this._lastPreviewSeq) {
        this._lastPreviewSeq = frame.sequence;
        this._previewItem.updateFrame(frame);
      }

      const status = await this._client.getStatus();
      this._lastStatus = status;
      this._updateCaption(status);
    }

    destroy() {
      this._stopPreviewPolling();
      super.destroy();
    }
  },
);

/**
 * GNOME Shell extension entry point.
 */
export default class Dp2Extension extends Extension {
  enable() {
    this._indicator = new Dp2Indicator(this.path);
    Main.panel.addToStatusArea("dp2-camera", this._indicator);
  }

  disable() {
    if (this._indicator) {
      this._indicator.destroy();
      this._indicator = null;
    }
  }
}
