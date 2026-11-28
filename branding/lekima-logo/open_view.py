"""Frame the saved scene for inspection when Blender opens it.

Run as: blender lekima-logo.blend --python open_view.py
"""

import bpy

for window in bpy.context.window_manager.windows:
    for area in window.screen.areas:
        if area.type != "VIEW_3D":
            continue
        space = area.spaces.active
        space.shading.type = "MATERIAL"
        for flag in ("use_scene_lights", "use_scene_world"):
            if hasattr(space.shading, flag):
                setattr(space.shading, flag, True)
        space.region_3d.view_perspective = "CAMERA"
        if hasattr(space.region_3d, "view_camera_zoom"):
            space.region_3d.view_camera_zoom = 0.0
        space.overlay.show_overlays = True

for obj in bpy.data.objects:
    obj.select_set(False)
