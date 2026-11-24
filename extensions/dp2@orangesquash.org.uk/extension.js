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

const Dp2Indicator = GObject.registerClass(
  class Dp2Indicator extends PanelMenu.Button {
    _init() {
      super._init(0.5, "Depth Pro 2 Camera", false);

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
      this._panelLabel = new St.Label({
        text: "DP2",
        y_align: Clutter.ActorAlign.CENTER,
        style_class: "dp2-panel-label",
      });
      panelBox.add_child(this._panelIcon);
      panelBox.add_child(this._panelLabel);
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

    _buildMenu() {
      this._previewItem = new PreviewMenuItem();
      this.menu.addMenuItem(this._previewItem);

      this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

      this._effectsSection = new PopupMenu.PopupMenuSection();
      this.menu.addMenuItem(this._effectsSection);

      this._optionsSection = new PopupMenu.PopupMenuSection();
      this.menu.addMenuItem(this._optionsSection);

      this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

      this._sourceMenu = new PopupMenu.PopupSubMenuMenuItem("Source Camera");
      this.menu.addMenuItem(this._sourceMenu);

      this._modelMenu = new PopupMenu.PopupSubMenuMenuItem("Depth Model");
      this.menu.addMenuItem(this._modelMenu);

      this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

      const resetItem = new PopupMenu.PopupMenuItem(
        "Reset Temporal Depth State",
      );
      resetItem.connect("activate", () => {
        this._client
          .resetDepthState()
          .catch((err) => logError(err, "dp2 reset depth failed"));
      });
      this.menu.addMenuItem(resetItem);

      const stopItem = new PopupMenu.PopupMenuItem(
        "Stop Virtual Camera Service",
      );
      stopItem.connect("activate", () => {
        this._client
          .shutdown()
          .then(() => {
            this._previewItem.showPlaceholder("Service stopped");
            this._previewItem.setCaption("Open menu to reconnect");
          })
          .catch((err) => logError(err, "dp2 shutdown failed"));
      });
      this.menu.addMenuItem(stopItem);
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
        this._rebuildEffects(effects, status.active_effect);
        this._rebuildOptions(status.active_effect);
        this._rebuildSources(sources, status.source_name);
        this._rebuildModels(models, status.active_model);
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
      if (typeof status.last_inference_ms === "number")
        parts.push(`${Math.round(status.last_inference_ms)} ms`);
      if (status.active_model) parts.push(status.active_model);
      if (status.source_name && status.source_name !== "none")
        parts.push(status.source_name);

      this._previewItem.setCaption(
        parts.length > 0
          ? parts.join(" · ")
          : "Ready — choose a camera or video",
      );
    }

    _rebuildEffects(effects, activeEffect) {
      this._effectsSection.removeAll();

      const header = new PopupMenu.PopupMenuItem("Effect", {
        reactive: false,
        can_focus: false,
        style_class: "dp2-section-header",
      });
      this._effectsSection.addMenuItem(header);

      for (const effect of effects) {
        const item = new PopupMenu.PopupMenuItem(effect.name);
        const isActive = activeEffect && activeEffect.id === effect.id;
        item.setOrnament(
          isActive ? PopupMenu.Ornament.DOT : PopupMenu.Ornament.NONE,
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
        this._addChipRow(
          "Palette",
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
        this._addChipRow(
          "Blur Strength",
          [
            { id: "4", label: "Soft" },
            { id: "8", label: "Medium" },
            { id: "14", label: "Strong" },
          ],
          params.blur_radius ?? "8",
          (radius) =>
            this._applyCustomParams(activeEffect, { blur_radius: radius }),
        );
        this._addChipRow(
          "Focus",
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
        this._addChipRow(
          "Cutoff",
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
        this._addChipRow(
          "Backdrop",
          [
            { id: "#1b3a57", label: "Studio Blue" },
            { id: "#1f2428", label: "Charcoal" },
            { id: "#00b140", label: "Chroma" },
            { id: "gradient", label: "Sunset" },
          ],
          params.background ?? "gradient",
          (bg) => this._applyCustomParams(activeEffect, { background: bg }),
        );
      }
    }

    _addChipRow(title, options, activeId, onSelect) {
      const rowItem = new PopupMenu.PopupBaseMenuItem({
        reactive: false,
        can_focus: false,
        style_class: "dp2-chip-item",
      });

      const box = new St.BoxLayout({
        orientation: Clutter.Orientation.VERTICAL,
        x_expand: true,
        style_class: "dp2-chip-container",
      });

      box.add_child(
        new St.Label({
          text: title,
          style_class: "dp2-chip-title",
        }),
      );

      const chips = new St.BoxLayout({
        x_expand: true,
        style_class: "dp2-chip-row",
      });

      for (const opt of options) {
        const btn = new St.Button({
          label: opt.label,
          can_focus: true,
          x_expand: true,
          style_class:
            opt.id === activeId
              ? "dp2-chip-button dp2-chip-active"
              : "dp2-chip-button",
        });
        btn.connect("clicked", () => {
          onSelect(opt.id).catch((err) =>
            logError(err, `dp2 chip ${opt.id} failed`),
          );
        });
        chips.add_child(btn);
      }

      box.add_child(chips);
      rowItem.add_child(box);
      this._optionsSection.addMenuItem(rowItem);
    }

    _rebuildSources(sources, activeSourceName) {
      this._sourceMenu.menu.removeAll();
      const activeLabel =
        activeSourceName && activeSourceName !== "none"
          ? `Source: ${activeSourceName}`
          : "Source Camera";
      this._sourceMenu.label.text = activeLabel;

      for (const src of sources) {
        const prefix = src.is_imported_video ? "[Video] " : "";
        const item = new PopupMenu.PopupMenuItem(`${prefix}${src.description}`);
        item.setOrnament(
          src.name === activeSourceName
            ? PopupMenu.Ornament.DOT
            : PopupMenu.Ornament.NONE,
        );
        item.connect("activate", () => {
          this.selectSource(src.name).catch((err) =>
            logError(err, "dp2 setSource failed"),
          );
        });
        this._sourceMenu.menu.addMenuItem(item);
      }

      if (sources.length > 0)
        this._sourceMenu.menu.addMenuItem(
          new PopupMenu.PopupSeparatorMenuItem(),
        );

      const chooseVideo = new PopupMenu.PopupMenuItem("Choose video…");
      chooseVideo.connect("activate", () => {
        this._chooseVideoFile().catch((err) =>
          logError(err, "dp2 choose video failed"),
        );
      });
      this._sourceMenu.menu.addMenuItem(chooseVideo);
    }

    _rebuildModels(models, activeModelId) {
      this._modelMenu.menu.removeAll();
      this._modelMenu.label.text = activeModelId
        ? `Model: ${activeModelId}`
        : "Depth Model";

      if (models.length === 0) {
        const empty = new PopupMenu.PopupMenuItem("No models discovered", {
          reactive: false,
        });
        this._modelMenu.menu.addMenuItem(empty);
        return;
      }

      for (const model of models) {
        const item = new PopupMenu.PopupMenuItem(
          `${model.name} (${model.size_label})`,
          {
            reactive: model.available,
          },
        );
        item.setOrnament(
          model.id === activeModelId
            ? PopupMenu.Ornament.DOT
            : PopupMenu.Ornament.NONE,
        );
        if (model.available) {
          item.connect("activate", () => {
            this.selectModel(model.id).catch((err) =>
              logError(err, "dp2 setModel failed"),
            );
          });
        }
        this._modelMenu.menu.addMenuItem(item);
      }
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
      const frame = await this._client.getPreview(280);
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
    this._indicator = new Dp2Indicator();
    Main.panel.addToStatusArea("dp2-camera", this._indicator);
  }

  disable() {
    if (this._indicator) {
      this._indicator.destroy();
      this._indicator = null;
    }
  }
}
