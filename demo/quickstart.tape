Output demo/quickstart.gif

Require pcx

Set Shell "bash"
Set Theme "Catppuccin Mocha"
Set FontSize 18
Set Width 1200
Set Height 840
Set Padding 24
Set WindowBar "Colorful"
Set TypingSpeed 18ms
Set Framerate 30

Type "pcx --version" Sleep 300ms Enter
Sleep 1s

Type "cloud=tests/fixtures/valid/pointcloud2.mcap" Sleep 300ms Enter
Sleep 500ms

Type "pcx topics $cloud" Sleep 300ms Enter
Sleep 2s

Type "pcx render $cloud --topic /lidar/points --frame 0 --width 32 --height 12" Sleep 300ms Enter
Sleep 2s

Type "pcx extract $cloud --topic /lidar/points --frame 0 --encoding ascii -o /tmp/frame.pcd --force" Sleep 300ms Enter
Sleep 1s

Type "sed -n '1,13p' /tmp/frame.pcd" Sleep 300ms Enter
Sleep 4s
