// Shared routing state between the drawing pipeline and the gesture
// handler (both listen on the same viewport element):
//
//  - pen / mouse-left  → draw (PointerPipeline)
//  - 1 finger          → pan (GestureHandler) — unless no pen has ever been
//                        used on this device, in which case a finger that
//                        lands on a page draws (phones without stylus)
//  - 2 fingers         → pinch zoom/pan (GestureHandler)
//  - palm while pen is down → ignored (palm rejection)
//
// The pipeline *claims* pointers it draws with; the gesture handler skips
// claimed pointers. Registration order of the listeners does not matter
// because gestures only commit to a pan after a movement threshold, by
// which point the claim is already visible.

const PEN_SEEN_KEY = 'ihn_pen_seen';

export class InputRouter {
  penEverSeen: boolean;
  /** Pointer ids currently used for drawing. */
  private claimed = new Set<number>();
  /** True while a pen stroke is in progress (palm rejection). */
  penDrawing = false;

  constructor() {
    let seen = false;
    try {
      seen = localStorage.getItem(PEN_SEEN_KEY) === '1';
    } catch {
      /* storage unavailable */
    }
    this.penEverSeen = seen;
  }

  notePen(): void {
    if (this.penEverSeen) return;
    this.penEverSeen = true;
    try {
      localStorage.setItem(PEN_SEEN_KEY, '1');
    } catch {
      /* ignore */
    }
  }

  claim(pointerId: number): void {
    this.claimed.add(pointerId);
  }

  release(pointerId: number): void {
    this.claimed.delete(pointerId);
  }

  isClaimed(pointerId: number): boolean {
    return this.claimed.has(pointerId);
  }

  claimedCount(): number {
    return this.claimed.size;
  }
}
