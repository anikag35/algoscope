# Scripted demo recording. Regenerate with:  vhs demo.tape
# Committed so the GIF can be rebuilt after a UI change instead of
# being re-performed by hand.

Output assets/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 1200
Set Height 680
Set Padding 16
Set Framerate 18
Set WindowBar Colorful
Set BorderRadius 8

# Launch off-camera so the GIF opens on the app, not on a shell prompt.
Hide
Type "./target/release/algoscope --seed 11 --size 32 --grid 32x14"
Enter
Sleep 1500ms
Show

# The shuffled array, at rest.
Sleep 1200ms

# Speed 3 -> 5 (160 steps/sec), or 32 items takes most of a minute.
Up
Up
Sleep 400ms

# Bubble sort, start to finish.
Space
Sleep 7s
Sleep 800ms

# Same array through insertion sort: the counters are a direct comparison.
Tab
Sleep 1200ms
Space
Sleep 5s
Sleep 1200ms

# Over to pathfinding on a weighted grid.
Type "m"
Sleep 1500ms

# Dijkstra: optimal, but searches blind in every direction.
Type "3"
Sleep 800ms
Up
Space
Sleep 6s
Sleep 1500ms

# Same map through A*: identical route, a fraction of the cells expanded.
Type "4"
Sleep 1000ms
Space
Sleep 4s
Sleep 2500ms
