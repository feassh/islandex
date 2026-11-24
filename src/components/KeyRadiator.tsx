import { forwardRef, useImperativeHandle, useRef } from "react";
import { COLLAPSED_BASE, ISLAND_TOP_OFFSET } from "../physics/spring";

export interface KeyRadiatorHandle {
  emitKey: (label: string, islandWidth: number, islandHeight: number) => void;
}

const MAX_ACTIVE_PARTICLES = 24;
// Exclude the top 60-degree cone centered straight up (-90 deg -> exclude [-120 deg, -60 deg]).
// Valid 300-degree radiation arc: [-60 deg, +240 deg] clockwise (+X right, +Y down).
const MIN_ANGLE_DEG = -60;
const ARC_SPAN_DEG = 300;
const DEG_TO_RAD = Math.PI / 180;

const ORIGIN_Y = ISLAND_TOP_OFFSET + COLLAPSED_BASE.height / 2; // 26px from top of host window
const SAFE_MIN_Y = 14; // Keeps the 22px tall keycap cleanly visible inside the top screen edge

export const KeyRadiator = forwardRef<KeyRadiatorHandle>(function KeyRadiator(_, ref) {
  const containerRef = useRef<HTMLDivElement>(null);
  const lastAngleRef = useRef<number | null>(null);

  useImperativeHandle(
    ref,
    () => ({
      emitKey(label: string, islandWidth: number, islandHeight: number) {
        const container = containerRef.current;
        if (!container || !label) return;

        // Trim oldest particles if typing very rapidly
        while (container.childElementCount >= MAX_ACTIVE_PARTICLES) {
          container.firstElementChild?.remove();
        }

        // Pick a random angle in [-60, 240] deg, nudged away from the immediately preceding key
        let angleDeg = MIN_ANGLE_DEG + Math.random() * ARC_SPAN_DEG;
        if (lastAngleRef.current !== null && Math.abs(angleDeg - lastAngleRef.current) < 26) {
          angleDeg =
            MIN_ANGLE_DEG +
            ((angleDeg - MIN_ANGLE_DEG + 48 + Math.random() * 140) % ARC_SPAN_DEG);
        }
        lastAngleRef.current = angleDeg;

        const rad = angleDeg * DEG_TO_RAD;
        const cos = Math.cos(rad);
        const sin = Math.sin(rad);

        // Start emerging from just inside the collapsed island's perimeter (works for circle & capsule)
        const rx = Math.max(8, islandWidth / 2 - 4);
        const ry = Math.max(8, islandHeight / 2 - 4);
        const startX = cos * rx * 0.65;
        const startY = sin * ry * 0.65;

        // Radiate closer to the island with gentle drift so each key stays snug and readable
        const travel = 24 + Math.random() * 18;
        const midX = cos * (rx + travel * 0.62);
        const midY = Math.max(SAFE_MIN_Y - ORIGIN_Y, sin * (ry + travel * 0.62));

        const endX = cos * (rx + travel);
        const rawEndY = sin * (ry + travel) + 6;
        const endY = Math.max(SAFE_MIN_Y - ORIGIN_Y, rawEndY);

        const tiltDeg = (Math.random() - 0.5) * 14;
        const duration = 1350 + Math.random() * 250;

        const el = document.createElement("span");
        el.className = "key-particle";
        if (label.length >= 3) {
          el.classList.add("key-particle-compact");
        }
        el.textContent = label;
        container.appendChild(el);

        const animation = el.animate(
          [
            {
              opacity: 0,
              transform: `translate3d(calc(-50% + ${startX.toFixed(1)}px), calc(-50% + ${startY.toFixed(1)}px), 0) scale(0.55) rotate(0deg)`,
              filter: "blur(1px)",
            },
            {
              opacity: 1,
              transform: `translate3d(calc(-50% + ${midX.toFixed(1)}px), calc(-50% + ${midY.toFixed(1)}px), 0) scale(1.03) rotate(${(tiltDeg * 0.45).toFixed(1)}deg)`,
              filter: "blur(0px)",
              offset: 0.16,
            },
            {
              opacity: 0.96,
              transform: `translate3d(calc(-50% + ${(midX + (endX - midX) * 0.48).toFixed(1)}px), calc(-50% + ${(midY + (endY - midY) * 0.48).toFixed(1)}px), 0) scale(0.98) rotate(${(tiltDeg * 0.78).toFixed(1)}deg)`,
              filter: "blur(0px)",
              offset: 0.62,
            },
            {
              opacity: 0,
              transform: `translate3d(calc(-50% + ${endX.toFixed(1)}px), calc(-50% + ${endY.toFixed(1)}px), 0) scale(0.86) rotate(${tiltDeg.toFixed(1)}deg)`,
              filter: "blur(1.2px)",
            },
          ],
          {
            duration,
            easing: "cubic-bezier(0.22, 1, 0.36, 1)",
            fill: "forwards",
          },
        );

        animation.onfinish = () => el.remove();
        animation.oncancel = () => el.remove();
      },
    }),
    [],
  );

  return <div ref={containerRef} className="key-radiator" aria-hidden="true" />;
});
