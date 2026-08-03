# Inhouse Notes v5.9.4

## Clear peer-to-peer connection state

- Shows a slow, softly animated blue halo around each remote-device profile photo while its direct peer-to-peer connection is being established or recovered.
- Replaces the halo with a compact blue chain badge at the bottom-left of the photo only after the remote device has an open RTCDataChannel.
- Keeps the existing avatar dimensions and the device/status badge unchanged, including the smaller mobile header layout.
- Continues showing the reconnecting halo when Drive presence briefly lags behind an active P2P recovery attempt.
- Respects reduced-motion preferences by using a static blue ring instead of an animation.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
