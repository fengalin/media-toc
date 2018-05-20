import bpy, os

xy = os.environ['XY']
for scene in bpy.data.scenes:
    scene.render.resolution_x = int(xy)
    scene.render.resolution_y = int(xy)
    scene.render.filepath = 'assets/icons/hicolor/%sx%s/apps/%s.png'%(xy, xy, os.environ['APP_ID'])
    scene.frame_end = 1
    bpy.ops.render.render(write_still=True)