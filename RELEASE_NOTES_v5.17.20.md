# v5.17.20

Smooth, immediate handwriting.

- Pen ink is drawn as a smooth curve through every measured sample again (centripetal Catmull-Rom spline) instead of straight segments between samples. Corners stay exactly where the pen was and no stored point changes.
- Live ink and page repaints use the same curve, so a stroke does not shift when you lift the pen or the page redraws. The background PDF renderer uses it too.
- The newest piece of the stroke is drawn ahead to the pen tip using the browser's predicted pen positions, removing the frame of lag behind the pen.
- Live strokes over Drive: a device that wrote in the last two minutes is checked every 300 ms instead of every 2 s after a pause, so the first stroke after a pause appears while it is being written.
- Web and Android web assets updated; APK/frozen installer unchanged.
