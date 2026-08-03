# Inhouse Notes v5.9.3

## Reliable peer-to-peer recovery

- Adds an encrypted rendezvous announcement inside the existing Drive document, so active devices can discover each other even when Drive presence is delayed or stale.
- Keeps a small local warm-start cache of previously verified devices and starts reconnecting to them as soon as the document opens.
- Rotates the rendezvous route epoch after a network change, allowing the other device to detect the change and recover even when its own browser misses the network event.
- Treats every Android network-change event as meaningful even when the browser still reports the same coarse Wi-Fi or 4G signature.
- Replaces stale channels with a verified sub-second probe after network changes, app resume, tab focus and WebView resume instead of waiting for the old multi-second grace period.
- Uses shorter bounded connection attempts and additional Google STUN endpoints, while keeping Drive as the durable automatic fallback.

## Faster signalling

- Reuses the verified encrypted signalling key after its race-safe confirmation instead of adding a Drive metadata request to every offer and ICE-candidate batch.
- Reduces the recent-comment scan window and retains paginated signalling reads, avoiding stale comment traffic without missing active offers.
- Preserves rendezvous state across transient presence refreshes and isolates in-flight announcements when changing documents.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
