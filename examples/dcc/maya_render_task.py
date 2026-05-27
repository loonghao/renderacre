import argparse
from pathlib import Path

import maya.standalone


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--frame", type=int, required=True)
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()

    maya.standalone.initialize(name="python")
    try:
        import maya.cmds as cmds

        cmds.file(new=True, force=True)
        cube = cmds.polyCube(name=f"RenderacreCube_{args.frame}")[0]
        cmds.rotate(0, args.frame * 10, 0, cube)
        cmds.directionalLight(name="RenderacreKeyLight", rotation=(-45, 30, 0))
        camera = cmds.camera(name="RenderacreCamera")[0]
        cmds.setAttr(f"{camera}.translate", 4, 5, 6, type="double3")
        cmds.setAttr(f"{camera}.rotate", -35, 35, 0, type="double3")
        cmds.lookThru(camera)

        output_dir = Path(args.output_dir)
        output_dir.mkdir(parents=True, exist_ok=True)
        scene_path = output_dir / f"maya_frame_{args.frame:04d}.ma"
        cmds.currentTime(args.frame)
        cmds.file(rename=str(scene_path))
        cmds.file(save=True, type="mayaAscii")
        print(f"saved {scene_path}")
    finally:
        maya.standalone.uninitialize()


if __name__ == "__main__":
    main()
