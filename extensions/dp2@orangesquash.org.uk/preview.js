// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

import Clutter from "gi://Clutter";
import Cogl from "gi://Cogl";
import GLib from "gi://GLib";
import GObject from "gi://GObject";
import St from "gi://St";

import * as PopupMenu from "resource:///org/gnome/shell/ui/popupMenu.js";

const PREVIEW_WIDTH = 288;
const PREVIEW_HEIGHT = 186;

/**
 * Inline live video preview card rendered inside the top-bar popover using
 * St.ImageContent.
 */
export const PreviewMenuItem = GObject.registerClass(
  class PreviewMenuItem extends PopupMenu.PopupBaseMenuItem {
    _init() {
      super._init({
        reactive: false,
        can_focus: false,
        style_class: "dp2-preview-item",
      });

      this._container = new St.BoxLayout({
        orientation: Clutter.Orientation.VERTICAL,
        x_expand: true,
        style_class: "dp2-preview-box",
      });

      this._imageContent = St.ImageContent.new_with_preferred_size(
        PREVIEW_WIDTH,
        PREVIEW_HEIGHT,
      );

      this._imageActor = new Clutter.Actor({
        width: PREVIEW_WIDTH,
        height: PREVIEW_HEIGHT,
        content: this._imageContent,
        content_gravity: Clutter.ContentGravity.RESIZE_ASPECT,
        visible: false,
      });

      const placeholderBox = new St.BoxLayout({
        orientation: Clutter.Orientation.VERTICAL,
        x_align: Clutter.ActorAlign.CENTER,
        y_align: Clutter.ActorAlign.CENTER,
        style_class: "dp2-preview-placeholder-box",
      });
      placeholderBox.add_child(
        new St.Icon({
          icon_name: "camera-video-symbolic",
          icon_size: 28,
          style_class: "dp2-preview-placeholder-icon",
        }),
      );
      this._placeholderLabel = new St.Label({
        text: "Waiting for video frame…",
        x_align: Clutter.ActorAlign.CENTER,
        style_class: "dp2-preview-placeholder",
      });
      placeholderBox.add_child(this._placeholderLabel);
      this._placeholder = placeholderBox;

      this._frameBin = new St.Bin({
        width: PREVIEW_WIDTH,
        height: PREVIEW_HEIGHT,
        x_align: Clutter.ActorAlign.CENTER,
        y_align: Clutter.ActorAlign.CENTER,
        style_class: "dp2-preview-frame",
        child: this._placeholder,
      });

      this._caption = new St.Label({
        text: "Connecting to dp2…",
        x_align: Clutter.ActorAlign.CENTER,
        style_class: "dp2-preview-caption",
      });

      this._container.add_child(this._frameBin);
      this._container.add_child(this._caption);
      this.add_child(this._container);
    }

    setCaption(text) {
      this._caption.text = text;
    }

    showPlaceholder(message) {
      if (this._frameBin.get_child() !== this._placeholder)
        this._frameBin.set_child(this._placeholder);
      this._placeholderLabel.text = message;
      this._imageActor.visible = false;
    }

    updateFrame(frame) {
      if (!frame || !frame.rgba_base64 || frame.width <= 0 || frame.height <= 0)
        return;

      const decoded = GLib.base64_decode(frame.rgba_base64);
      const expectedLen = frame.width * frame.height * 4;
      if (decoded.length < expectedLen) return;

      const coglContext = global.stage.context.get_backend().get_cogl_context();
      const bytes = GLib.Bytes.new(decoded);
      this._imageContent.set_bytes(
        coglContext,
        bytes,
        Cogl.PixelFormat.RGBA_8888,
        frame.width,
        frame.height,
        frame.width * 4,
      );

      const aspectHeight = Math.max(
        135,
        Math.min(
          PREVIEW_HEIGHT,
          Math.round((PREVIEW_WIDTH * frame.height) / frame.width),
        ),
      );
      this._imageActor.set_size(PREVIEW_WIDTH, aspectHeight);
      this._frameBin.set_size(PREVIEW_WIDTH, aspectHeight);
      this._imageActor.visible = true;
      if (this._frameBin.get_child() !== this._imageActor)
        this._frameBin.set_child(this._imageActor);
    }
  },
);
