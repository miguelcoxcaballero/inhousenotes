# Inhouse Notes v5.11.8

- Shows every pixel identified as part of the lower yellow box in bright yellow from corner detection through frame tracing, mesh warping and colour correction.
- Keeps the detected yellow overlay visible until the real stencil is ready, avoiding an abrupt gap at the end of processing.
- Uses the four printed colour circles as per-page red, black, blue and green references instead of applying a single global colour matrix.
- Uses multiple yellow-frame samples and the sheet itself as yellow and white references, producing clean white paper and more vivid, brush-matched ink tones.
- Separately traces both rails of the lower square box so its geometry and animated mask remain stable under perspective and page curvature.
