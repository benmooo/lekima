"""Build and render the LEKIMA identity in Blender.

The logo is not a mood piece: it is the project's own visual language made
physical.  The wordmark is extruded from the `WORDMARK` block art in
`crates/lekima-tui/src/ui.rs` (the empty-state brand art), the two-tone split
(`LEK` accent, `IMA` accent-alt) matches that constant, and every colour comes
from the palette parser reading `crates/lekima-tui/src/theme.rs`.  The slab is
the console: brand chip, dim rule, `//` label, 12-cell rail, version silkscreen.

Usage (from the repository root):

    blender -b --factory-startup --python branding/lekima-logo/logo.py -- \
        --scene hero --theme cyber

    blender -b --factory-startup --python branding/lekima-logo/logo.py -- \
        --scene loop --theme cyber --res 1920x1080 --frames 120

Scenes: hero (16:9 still), alpha (transparent lockup), square (1:1 mark),
loop (seamless 4 s animation written as H.264).
"""

from __future__ import annotations

import argparse
import math
import os
import re
import shutil
import subprocess
import sys

import bpy
from mathutils import Vector

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
THEME_RS = os.path.join(ROOT, "crates", "lekima-tui", "src", "theme.rs")
UI_RS = os.path.join(ROOT, "crates", "lekima-tui", "src", "ui.rs")
CARGO_TOML = os.path.join(ROOT, "Cargo.toml")

FONT_CANDIDATES = [
    "/System/Library/Fonts/SFNSMono.ttf",
    "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
]

FPS = 30
LOOP_FRAMES = 120  # 4 s at 30 fps


# --------------------------------------------------------------------------
# Repository sources: palettes, wordmark, version
# --------------------------------------------------------------------------


def _read(path: str) -> str:
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def load_palettes() -> dict[str, dict[str, tuple[int, int, int]]]:
    """Parse every `pub const fn <name>() -> Self` palette out of theme.rs."""
    source = _read(THEME_RS)
    palettes: dict[str, dict[str, tuple[int, int, int]]] = {}
    blocks = re.finditer(r"pub const fn (\w+)\(\) -> Self \{\s*Self \{(.*?)\n        \}", source, re.S)
    for block in blocks:
        tokens = {
            key: (int(r), int(g), int(b))
            for key, r, g, b in re.findall(r"(\w+): Color::Rgb\((\d+),\s*(\d+),\s*(\d+)\)", block.group(2))
        }
        if "accent" in tokens:
            palettes[block.group(1)] = tokens
    return palettes


def load_wordmark() -> list[str]:
    """Read the shipped empty-state wordmark: three rows, two tinted halves."""
    source = _read(UI_RS)
    match = re.search(r"const WORDMARK: \[\[&str; 2\]; 3\] = \[(.*?)\];", source, re.S)
    if not match:
        raise SystemExit("WORDMARK not found in ui.rs")
    rows = re.findall(r'\["([^"]*)", "([^"]*)"\]', match.group(1))
    if len(rows) != 3:
        raise SystemExit("WORDMARK is not three rows")
    return [left + right for left, right in rows]


def load_version() -> str:
    match = re.search(r'^version\s*=\s*"([^"]+)"', _read(CARGO_TOML), re.M)
    return match.group(1) if match else "0.0.0"


def hex_to_linear(rgb: tuple[int, int, int]) -> tuple[float, float, float, float]:
    def channel(value: int) -> float:
        c = value / 255.0
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4

    return (channel(rgb[0]), channel(rgb[1]), channel(rgb[2]), 1.0)


# `▀` is the top half of a cell, `▄` the bottom half, `█` both.
HALVES = {
    "█": (True, True),
    "▀": (True, False),
    "▄": (False, True),
    " ": (False, False),
}


def wordmark_cells(rows: list[str], split: int = 12) -> list[tuple[int, int, bool]]:
    """Return (column, sub-row, is_right_half) for every inked sub-cell."""
    cells = []
    for row_index, row in enumerate(rows):
        for column, glyph in enumerate(row):
            top, bottom = HALVES.get(glyph, (False, False))
            if top:
                cells.append((column, row_index * 2, column >= split))
            if bottom:
                cells.append((column, row_index * 2 + 1, column >= split))
    return cells


# --------------------------------------------------------------------------
# Scene plumbing
# --------------------------------------------------------------------------


def reset_scene() -> bpy.types.Scene:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.fps = FPS
    scene.frame_start = 1
    scene.frame_end = LOOP_FRAMES
    return scene


def try_set(target, name: str, value) -> bool:
    try:
        setattr(target, name, value)
        return True
    except (AttributeError, TypeError, ValueError):
        return False


def link(obj):
    bpy.context.scene.collection.objects.link(obj)
    return obj


def action_fcurves(obj) -> list:
    """F-curves of an object's action, across the 4.x and 5.x action APIs."""
    data = getattr(obj, "animation_data", None)
    action = getattr(data, "action", None) if data else None
    if action is None:
        return []
    legacy = getattr(action, "fcurves", None)
    if legacy is not None:
        return list(legacy)
    curves = []
    for layer in getattr(action, "layers", []):
        for strip in getattr(layer, "strips", []):
            for slot in getattr(action, "slots", []):
                try:
                    bag = strip.channelbag(slot)
                except (TypeError, KeyError):
                    continue
                if bag is not None:
                    curves.extend(bag.fcurves)
    return curves


def make_material(name: str, color, *, emission: float = 0.0, roughness: float = 0.45,
                  metallic: float = 0.0, coat: float = 0.0) -> bpy.types.Material:
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    bsdf = nodes.get("Principled BSDF")
    if bsdf is None:
        return material
    inputs = bsdf.inputs
    if "Base Color" in inputs:
        inputs["Base Color"].default_value = color
    if "Metallic" in inputs:
        inputs["Metallic"].default_value = metallic
    if "Roughness" in inputs:
        inputs["Roughness"].default_value = roughness
    if "Coat Weight" in inputs:
        inputs["Coat Weight"].default_value = coat
    if "Coat Roughness" in inputs:
        inputs["Coat Roughness"].default_value = 0.08
    if emission and "Emission Color" in inputs:
        inputs["Emission Color"].default_value = color
        inputs["Emission Strength"].default_value = emission
    return material


def make_emission(name: str, color, strength: float) -> bpy.types.Material:
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    nodes.clear()
    output = nodes.new("ShaderNodeOutputMaterial")
    emission = nodes.new("ShaderNodeEmission")
    emission.inputs["Color"].default_value = color
    emission.inputs["Strength"].default_value = strength
    material.node_tree.links.new(emission.outputs["Emission"], output.inputs["Surface"])
    return material


def box(name: str, size, location, material, *, bevel: float = 0.0, rotation=(0, 0, 0)):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=location, rotation=rotation)
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = (size[0], size[1], size[2])
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    if bevel > 0:
        modifier = obj.modifiers.new("bevel", "BEVEL")
        modifier.width = bevel
        modifier.segments = 2
        modifier.limit_method = "ANGLE"
    obj.data.materials.append(material)
    return obj


def front_slot(obj, front_material, side_material, axis: int = 1, sign: int = -1):
    """Give the camera-facing faces one material and the rest another."""
    obj.data.materials.clear()
    obj.data.materials.append(front_material)
    obj.data.materials.append(side_material)
    for polygon in obj.data.polygons:
        polygon.material_index = 0 if polygon.normal[axis] * sign > 0.5 else 1


def text_mesh(name: str, body: str, size: float, location, material,
              *, spacing: float = 1.0, align: str = "LEFT", extrude: float = 0.02,
              offset: float = 0.0):
    curve = bpy.data.curves.new(name, "FONT")
    curve.body = body
    curve.font = load_font()
    curve.size = size
    curve.space_character = spacing
    curve.align_x = align
    curve.align_y = "CENTER"
    curve.extrude = extrude
    curve.offset = offset
    curve.resolution_u = 16
    obj = bpy.data.objects.new(name, curve)
    link(obj)
    obj.location = location
    obj.rotation_euler = (math.pi / 2, 0, 0)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.convert(target="MESH")
    obj.data.materials.append(material)
    return obj


def load_font():
    for path in FONT_CANDIDATES:
        if os.path.exists(path):
            try:
                return bpy.data.fonts.load(path)
            except RuntimeError:
                continue
    return bpy.data.fonts.load(os.path.join(bpy.utils.system_resource("DATAFILES"), "fonts"))


# --------------------------------------------------------------------------
# Model
# --------------------------------------------------------------------------


def wordmark_grid(rows: list[str], cell: float) -> tuple[list[tuple[int, int, bool]], float, float]:
    cells = wordmark_cells(rows)
    columns = max(len(row) for row in rows)
    height = len(rows) * 2 * cell
    return cells, columns * cell, height


def build_wordmark(rows: list[str], tokens, cell: float = 0.5, depth: float = 0.42,
                   origin=(0.0, 0.0, 0.0)):
    """Extrude the shipped block art into backlit segmented keys."""
    cells, width, height = wordmark_grid(rows, cell)
    gap = cell * 0.035
    key = cell - gap
    front_left = make_emission("key_accent", hex_to_linear(tokens["accent"]), 1.15)
    front_right = make_emission("key_accent_alt", hex_to_linear(tokens["accent_alt"]), 1.05)
    side = make_material("key_side", hex_to_linear(tokens["ground"]), roughness=0.42, metallic=0.25)

    collection = bpy.data.collections.new("wordmark")
    bpy.context.scene.collection.children.link(collection)
    for index, (column, sub_row, right) in enumerate(cells):
        x = origin[0] - width / 2 + cell / 2 + column * cell
        z = origin[2] + height / 2 - cell / 2 - sub_row * cell
        obj = box(
            f"key_{index:03d}",
            (key, depth, key),
            (x, origin[1] - depth / 2, z),
            side,
            bevel=cell * 0.018,
        )
        front_slot(obj, front_right if right else front_left, side)
        for other in list(obj.users_collection):
            other.objects.unlink(obj)
        collection.objects.link(obj)
    return width, height


def build_rail(tokens, origin, cells: int = 12, cell_width: float = 0.8, gap: float = 0.1):
    """12-cell progress rail; filled cells are stepped on during the loop."""
    empty = make_material("rail_empty", hex_to_linear(tokens["meter_empty"]), roughness=0.5)
    filled = make_emission("rail_fill", hex_to_linear(tokens["accent"]), 3.2)
    step = cell_width + gap
    start = origin[0]
    for index in range(cells):
        x = start + index * step + cell_width / 2
        obj = box(f"rail_{index:02d}", (cell_width, 0.03, 0.16), (x, origin[1], origin[2]), empty)
        fill = box(f"rail_fill_{index:02d}", (cell_width, 0.02, 0.18), (x, origin[1] - 0.03, origin[2]),
                   filled)
        # Discrete HUD fill: a cell is off or on, never a gradient.
        fill.scale = (0.0, 1.0, 1.0)
        fill.keyframe_insert("scale", frame=1)
        fill.scale = (1.0, 1.0, 1.0)
        fill.keyframe_insert("scale", frame=2 + index * (LOOP_FRAMES - 8) / cells)
        fill.scale = (1.0, 1.0, 1.0)
        fill.keyframe_insert("scale", frame=LOOP_FRAMES + 1)
        for fcurve in action_fcurves(fill):
            for keyframe in fcurve.keyframe_points:
                keyframe.interpolation = "CONSTANT"
    return cells * cell_width + (cells - 1) * gap


def corner_ticks(tokens, width: float, height: float, y: float, length: float = 0.55,
                 thickness: float = 0.045):
    accent = make_emission("tick", hex_to_linear(tokens["accent"]), 0.9)
    hx, hz = width / 2, height / 2
    for sx in (-1, 1):
        for sz in (-1, 1):
            x = sx * hx
            z = sz * hz
            box("tick_h", (length, 0.02, thickness), (x - sx * length / 2, y, z), accent)
            box("tick_v", (thickness, 0.02, length), (x, y, z - sz * length / 2), accent)


def text_width(obj) -> tuple[float, float]:
    xs = [vertex.co.x for vertex in obj.data.vertices]
    ys = [vertex.co.y for vertex in obj.data.vertices]
    return (max(xs) - min(xs), max(ys) - min(ys))


def mono_advance(size: float, spacing: float) -> float:
    """Advance width of one monospace cell, including letter spacing."""
    probe = text_mesh("probe", "MMMMMMMMMM", size, (0, 0, 0),
                      bpy.data.materials.new("probe"), spacing=spacing, extrude=0.0)
    width, _ = text_width(probe)
    curve = probe.data
    bpy.data.objects.remove(probe, do_unlink=True)
    bpy.data.meshes.remove(curve)
    return width / 10.0


def text_row(parts, x: float, z: float, size: float, y: float, *, spacing: float = 1.0,
             align: str = "left", extrude: float = 0.012):
    """Lay out mixed-colour segments on the monospace grid."""
    advance = mono_advance(size, spacing)
    made = []
    cursor = 0.0
    for body, material in parts:
        obj = text_mesh(f"seg_{len(made)}", body, size, (cursor, y, z), material,
                        spacing=spacing, align="LEFT", extrude=extrude)
        obj.location.x = cursor
        cursor += len(body) * advance
        made.append(obj)
    shift = x - cursor if align == "right" else x
    for obj in made:
        obj.location.x += shift
    return cursor


def build_slab(tokens, rows: list[str], *, with_slab: bool = True):
    """The console slab and its silkscreen: chip, label, rule, rail, version."""
    version = load_version()
    base = hex_to_linear(tokens["ground"])
    luminance = 0.2126 * base[0] + 0.7152 * base[1] + 0.0722 * base[2]
    slab_rgb = (min(base[0] * (0.92 if luminance > 0.2 else 3.0), 0.9),) + tuple(
        min(channel * (0.92 if luminance > 0.2 else 3.0), 0.9) for channel in base[1:3])
    ground = make_material("slab", slab_rgb + (1.0,), roughness=0.66, metallic=0.0)
    panel = make_material("silkscreen", hex_to_linear(tokens["panel"]), emission=0.5, roughness=0.5)
    muted = make_material("muted", hex_to_linear(tokens["muted"]), emission=0.4, roughness=0.5)
    dim = make_material("rule", hex_to_linear(tokens["dim"]), roughness=0.55)
    badge = make_material("chip", hex_to_linear(tokens["badge_bg"]), emission=1.5, roughness=0.4)
    badge_ink = make_material("chip_ink", (0.002, 0.002, 0.002, 1.0), roughness=1.0)
    ink = badge_ink.node_tree.nodes.get("Principled BSDF")
    if ink and "Specular IOR Level" in ink.inputs:
        ink.inputs["Specular IOR Level"].default_value = 0.0

    width, height, thickness = 15.6, 7.6, 0.55
    if with_slab:
        box("slab", (width, thickness, height), (0, thickness / 2, 0), ground, bevel=0.05)
    front = -0.002

    # Brand chip: inverted badge, exactly the HUD's ` LEKIMA ` chip.
    chip_w, chip_h = 2.0, 0.46
    chip_x, chip_z = -6.05, 2.68
    box("chip", (chip_w, 0.05, chip_h), (chip_x, front - 0.025, chip_z), badge, bevel=0.02)
    text_mesh("chip_text", "LEKIMA", 0.28, (chip_x, front - 0.055, chip_z - 0.004), badge_ink,
              spacing=1.2, align="CENTER", extrude=0.012, offset=0.006)

    # `// CONSOLE`, the section-label vocabulary from the TUI.
    text_row([("//", panel), (" CONSOLE", muted)], 6.86, 2.68, 0.27, front - 0.02,
             spacing=1.24, align="right")

    # Wordmark, centred, sitting proud of the slab.
    wordmark_width, wordmark_height = build_wordmark(rows, tokens, origin=(0.0, front, 0.78))

    # Identity line, then the dim rule and the rail.
    text_row([("NETEASE CLOUD MUSIC ", muted), (" //", panel), (" CONSOLE", muted)],
             -6.82, -1.35, 0.3, front - 0.02, spacing=1.22)
    box("rule", (13.64, 0.02, 0.028), (0, front - 0.012, -1.82), dim)

    build_rail(tokens, (-6.82, front - 0.02, -2.62))
    text_mesh("version", f"v{version}", 0.24, (6.82, front - 0.02, -2.62), muted,
              spacing=1.12, align="RIGHT", extrude=0.01)

    if with_slab:
        corner_ticks(tokens, width - 0.36, height - 0.36, front - 0.015)

    # Cursor: `▌`, blinking at the 2 Hz the HUD uses while typing.
    cursor = box("cursor", (0.09, 0.06, 0.3), (7.16, front - 0.03, 2.68), badge)
    for frame in range(1, LOOP_FRAMES + 1, 8):
        lit = (frame // 8) % 2 == 0
        cursor.scale.x = 0.09 if lit else 0.0001
        cursor.keyframe_insert("scale", index=0, frame=frame)
    for fcurve in action_fcurves(cursor):
        for keyframe in fcurve.keyframe_points:
            keyframe.interpolation = "CONSTANT"

    return {"width": width, "height": height, "thickness": thickness,
            "wordmark": (wordmark_width, wordmark_height)}


def build_studio(tokens, *, floor: bool = True):
    ground = hex_to_linear(tokens["ground"])
    world = bpy.data.worlds.new("world")
    world.use_nodes = True
    background = world.node_tree.nodes.get("Background")
    if background:
        background.inputs["Color"].default_value = ground
        background.inputs["Strength"].default_value = 0.35
    bpy.context.scene.world = world

    key = bpy.data.lights.new("key", "AREA")
    key.energy = 900
    key.size = 11.0
    key.color = (1.0, 0.985, 0.96)
    key_obj = link(bpy.data.objects.new("key", key))
    key_obj.location = (-4.5, -13.0, 7.5)
    key_obj.rotation_euler = (math.radians(30), 0, math.radians(-24))

    rim = bpy.data.lights.new("rim", "AREA")
    rim.energy = 650
    rim.size = 5.0
    rim.color = hex_to_linear(tokens["accent_alt"])[:3]
    rim_obj = link(bpy.data.objects.new("rim", rim))
    rim_obj.location = (8.0, 3.0, 4.5)
    rim_obj.rotation_euler = (math.radians(72), 0, math.radians(126))

    fill = bpy.data.lights.new("fill", "AREA")
    fill.energy = 180
    fill.size = 12.0
    fill.color = hex_to_linear(tokens["text"])[:3]
    fill_obj = link(bpy.data.objects.new("fill", fill))
    fill_obj.location = (0.0, -12.0, -2.5)
    fill_obj.rotation_euler = (math.radians(96), 0, 0)

    if floor:
        floor_mat = make_material("floor", (0.0035, 0.005, 0.007, 1.0), roughness=0.5,
                                  metallic=0.0)
        box("floor", (80, 80, 0.2), (0, 0, -5.2), floor_mat)

    # Slow key-light drift gives the still-life some life without a gradient.
    key_obj.location.x = -4.5
    key_obj.keyframe_insert("location", index=0, frame=1)
    key_obj.location.x = -3.6
    key_obj.keyframe_insert("location", index=0, frame=LOOP_FRAMES)
    for fcurve in action_fcurves(key_obj):
        for keyframe in fcurve.keyframe_points:
            keyframe.interpolation = "BEZIER"


def build_camera(scene, width: float, height: float, *, distance: float = 42.0,
                 lens: float = 85.0, yaw_degrees: float = 5.0, pitch_degrees: float = 3.0):
    camera_data = bpy.data.cameras.new("camera")
    camera_data.lens = lens
    camera_data.sensor_width = 36.0
    camera_data.dof.use_dof = True
    camera_data.dof.focus_distance = distance
    camera_data.dof.aperture_fstop = 5.6
    camera = link(bpy.data.objects.new("camera", camera_data))
    scene.camera = camera

    yaw = math.radians(yaw_degrees)
    pitch = math.radians(pitch_degrees)
    target = Vector((0.0, 0.0, 0.15))
    camera.location = target + Vector((math.sin(yaw) * distance,
                                       -math.cos(yaw) * distance,
                                       math.sin(pitch) * distance))
    direction = target - camera.location
    camera.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()

    # Two-degree drift over the loop: parallax, not a spin. Rotation is keyed on
    # every pose, otherwise the camera translates without re-aiming and the
    # panel slides out of frame.
    camera.keyframe_insert("location", frame=1)
    camera.keyframe_insert("rotation_euler", frame=1)
    camera.location = target + Vector((math.sin(yaw + math.radians(2.2)) * distance,
                                       -math.cos(yaw + math.radians(2.2)) * distance,
                                       math.sin(pitch) * distance))
    direction = target - camera.location
    camera.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
    camera.keyframe_insert("location", frame=LOOP_FRAMES)
    camera.keyframe_insert("rotation_euler", frame=LOOP_FRAMES)


def setup_render(scene, res: tuple[int, int], samples: int, engine: str, alpha: bool):
    scene.render.engine = "BLENDER_EEVEE" if engine == "eevee" else "CYCLES"
    scene.render.resolution_x, scene.render.resolution_y = res
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = alpha
    scene.render.image_settings.color_mode = "RGBA" if alpha else "RGB"

    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.view_settings.exposure = 0.0

    if engine == "eevee":
        eevee = scene.eevee
        try_set(eevee, "taa_render_samples", samples)
        try_set(eevee, "use_raytracing", True)
        try_set(eevee, "use_shadows", True)
        try_set(eevee, "use_motion_blur", True)
        try_set(eevee, "motion_blur_shutter", 0.35)
    else:
        scene.cycles.samples = samples
        scene.cycles.use_denoising = True
        prefs = bpy.context.preferences.addons["cycles"].preferences
        try:
            prefs.compute_device_type = "METAL"
            prefs.get_devices()
            for device in prefs.devices:
                device.use = device.type == "METAL"
            scene.cycles.device = "GPU"
        except Exception:
            scene.cycles.device = "CPU"

    setup_glare(scene)


def setup_glare(scene):
    """Fog glow on the emissive keys; subtle, so the type stays sharp."""
    scene.use_nodes = True
    tree = getattr(scene, "node_tree", None)
    if tree is None:
        return
    tree.nodes.clear()
    layers = tree.nodes.new("CompositorNodeRLayers")
    glare = tree.nodes.new("CompositorNodeGlare")
    glare.glare_type = "FOG_GLOW"
    try_set(glare, "quality", "HIGH")
    try_set(glare, "threshold", 1.1)
    try_set(glare, "size", 7)
    try_set(glare, "mix", -0.72)
    composite = tree.nodes.new("CompositorNodeComposite")
    tree.links.new(layers.outputs["Image"], glare.inputs["Image"])
    tree.links.new(glare.outputs["Image"], composite.inputs["Image"])


# --------------------------------------------------------------------------
# Scenes
# --------------------------------------------------------------------------


def scene_hero(tokens, rows, args):
    build_studio(tokens)
    build_slab(tokens, rows)
    build_camera(bpy.context.scene, 15.6, 7.6, distance=41.0, pitch_degrees=2.4)


def scene_alpha(tokens, rows):
    """Transparent lockup: geometry and silkscreen only, no slab or floor."""
    build_studio(tokens, floor=False)
    build_slab(tokens, rows, with_slab=False)
    build_camera(bpy.context.scene, 15.6, 7.6, distance=41.0, pitch_degrees=2.4)


def scene_square(tokens, rows):
    """1:1 mark: the `L` of the wordmark on a square console tile."""
    build_studio(tokens, floor=True)
    cell = 0.68
    left = [row[:3] for row in rows]
    slab_w, slab_h = 5.4, 5.4
    base = hex_to_linear(tokens["ground"])
    luminance = 0.2126 * base[0] + 0.7152 * base[1] + 0.0722 * base[2]
    factor = 0.92 if luminance > 0.2 else 3.0
    ground = make_material("slab", tuple(min(channel * factor, 0.9) for channel in base[:3]) + (1.0,),
                           roughness=0.66, metallic=0.0)
    box("slab", (slab_w, 0.6, slab_h), (0, 0.3, 0), ground, bevel=0.06)
    build_wordmark(left, tokens, cell=cell, origin=(0, 0, 0.0))
    corner_ticks(tokens, slab_w - 0.74, slab_h - 0.74, -0.036)
    build_camera(bpy.context.scene, slab_w, slab_h, distance=16.5, lens=85, pitch_degrees=2.0,
                 yaw_degrees=3.5)


def scene_loop(tokens, rows):
    scene_hero(tokens, rows, None)


# --------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------


def parse_args(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--scene", default="hero", choices=["hero", "alpha", "square", "loop"])
    parser.add_argument("--theme", default="cyber")
    parser.add_argument("--engine", default="eevee", choices=["eevee", "cycles"])
    parser.add_argument("--samples", type=int, default=64)
    parser.add_argument("--res", default=None)
    parser.add_argument("--frames", type=int, default=LOOP_FRAMES)
    parser.add_argument("--out", default=None)
    parser.add_argument("--preview", action="store_true")
    parser.add_argument("--save", default=None, help="write an editable .blend")
    parser.add_argument("--render", action="store_true", help="render even when --save is set")
    return parser.parse_args(argv)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    args = parse_args(argv)

    palettes = load_palettes()
    required = ("accent", "accent_alt", "text", "muted", "dim", "ground", "panel",
                "badge_bg", "badge_fg", "meter_empty")
    usable = {name: tokens for name, tokens in palettes.items()
              if all(role in tokens for role in required)}
    if args.theme not in usable:
        raise SystemExit(f"theme {args.theme!r} has no RGB roles for a render; "
                         f"choose one of {', '.join(sorted(usable))}")
    tokens = usable[args.theme]
    rows = load_wordmark()

    scene = reset_scene()
    if args.scene == "square":
        scene_square(tokens, rows)
    elif args.scene == "alpha":
        scene_alpha(tokens, rows)
    elif args.scene == "loop":
        scene_loop(tokens, rows)
    else:
        scene_hero(tokens, rows, args)

    if args.preview:
        res = (900, 900) if args.scene == "square" else (1280, 720)
        samples = min(args.samples, 24)
    else:
        res = (3840, 2160) if args.scene in ("hero", "alpha") else (2048, 2048)
        if args.scene == "loop":
            res = (1920, 1080)
        samples = args.samples
    if args.res:
        width, height = args.res.lower().split("x")
        res = (int(width), int(height))

    setup_render(scene, res, samples, args.engine, alpha=args.scene == "alpha")

    out_dir = args.out or os.path.join(HERE, "out")
    os.makedirs(out_dir, exist_ok=True)

    # An editable scene: camera at the hero pose, 4K frame, no render needed.
    if args.save:
        scene.frame_set(96)
        bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(args.save))
        print("SAVED", args.save)
        if not args.render:
            return

    if args.scene == "loop":
        scene.frame_end = args.frames
        scene.render.image_settings.file_format = "PNG"
        scene.render.image_settings.color_mode = "RGB"
        frames_dir = os.path.join(out_dir, "frames", args.theme)
        if os.path.isdir(frames_dir):
            shutil.rmtree(frames_dir)
        os.makedirs(frames_dir, exist_ok=True)
        scene.render.filepath = os.path.join(frames_dir, "frame_")
    else:
        scene.render.image_settings.file_format = "PNG"
        scene.frame_set(119 if args.scene == "alpha" else 96)
        name = {
            "hero": f"lekima-logo-hero-{args.theme}.png",
            "alpha": f"lekima-logo-alpha-{args.theme}.png",
            "square": f"lekima-logo-square-{args.theme}.png",
        }[args.scene]
        scene.render.filepath = os.path.join(out_dir, name)

    bpy.ops.render.render(write_still=args.scene != "loop", animation=args.scene == "loop")

    if args.scene == "loop":
        movie = os.path.join(out_dir, f"lekima-logo-loop-{args.theme}.mp4")
        encode = [
            "ffmpeg", "-y", "-loglevel", "error", "-framerate", "30", "-start_number", "1",
            "-i", os.path.join(frames_dir, "frame_%04d.png"),
            "-c:v", "libx264", "-preset", "slow", "-crf", "15", "-pix_fmt", "yuv420p",
            "-movflags", "+faststart", movie,
        ]
        try:
            subprocess.run(encode, check=True)
            print("WROTE", movie)
        except (OSError, subprocess.CalledProcessError) as error:
            print("Frames in", frames_dir, "- ffmpeg failed:", error)
    else:
        print("WROTE", scene.render.filepath)


if __name__ == "__main__":
    main()
