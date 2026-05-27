import argparse
from pathlib import Path

import bpy


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--frame", type=int, required=True)
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()

    bpy.ops.object.delete()
    bpy.ops.mesh.primitive_cube_add(size=2)
    cube = bpy.context.object
    cube.name = f"RenderacreCube_{args.frame}"
    cube.rotation_euler[2] = args.frame * 0.2

    bpy.ops.object.light_add(type="AREA", location=(0, -4, 5))
    bpy.context.object.data.energy = 450
    bpy.ops.object.camera_add(location=(4, -6, 4), rotation=(1.1, 0.0, 0.6))
    bpy.context.scene.camera = bpy.context.object

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    bpy.context.scene.frame_set(args.frame)
    bpy.context.scene.render.filepath = str(output_dir / f"blender_frame_{args.frame:04d}.png")
    bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    main()
