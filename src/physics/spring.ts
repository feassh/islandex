export interface SpringConfig {
  stiffness: number;
  damping: number;
  mass: number;
}

const MAX_DT = 0.05; // Clamp frame deltas to 50ms to prevent instability after sleep
const FIXED_SUBSTEP = 1 / 240; // 240Hz physics sub-stepping for frame-rate independent springs

export class SpringAxis {
  public value: number;
  public velocity = 0;
  public target: number;
  private config: SpringConfig;
  private precision: number;

  constructor(initial: number, config: SpringConfig, precision = 0.01) {
    this.value = initial;
    this.target = initial;
    this.config = config;
    this.precision = precision;
  }

  public setTarget(target: number, config?: SpringConfig): void {
    this.target = target;
    if (config) {
      this.config = config;
    }
  }

  /**
   * Advances the damped harmonic oscillator by `dt` seconds using fixed substeps:
   * a = (-k * (x - target) - c * v) / m
   */
  public step(dt: number): void {
    let remaining = Math.min(Math.max(dt, 0), MAX_DT);
    const { stiffness, damping, mass } = this.config;

    while (remaining > 0) {
      const h = Math.min(remaining, FIXED_SUBSTEP);
      const displacement = this.value - this.target;
      const force = -stiffness * displacement - damping * this.velocity;
      const acceleration = force / mass;

      // Semi-implicit Euler (symplectic): preserves energy behavior and stays unconditionally stable
      this.velocity += acceleration * h;
      this.value += this.velocity * h;
      remaining -= h;
    }

    if (this.isAtRest()) {
      this.value = this.target;
      this.velocity = 0;
    }
  }

  public addImpulse(deltaVelocity: number, maxVelocity: number): void {
    this.velocity = Math.min(Math.max(this.velocity + deltaVelocity, -maxVelocity), maxVelocity);
  }

  public isAtRest(): boolean {
    return (
      Math.abs(this.value - this.target) < this.precision &&
      Math.abs(this.velocity) < this.precision * 10
    );
  }
}

export interface IslandMetrics {
  width: number;
  height: number;
  radius: number;
  avatarSize: number;
}

export interface IslandFrame {
  width: number;
  height: number;
  radius: number;
  avatarSize: number;
  scaleX: number;
  scaleY: number;
  translateY: number;
  settled: boolean;
}

export const ISLAND_HOST_WIDTH = 520;
export const ISLAND_TOP_OFFSET = 10;

// Compact circular default (32x32 with 30px SVG -> 24px white bloub inside a 4px black bezel).
// When right-only content (lyrics / prompt) is present, left padding stays 1px so the 24px white
// circle remains strictly concentric with the 16px left semicircle of the capsule.
export const COLLAPSED_BASE = {
  height: 32,
  avatarSize: 30,
  circlePaddingX: 1, // 30 + 2 = 32 -> exact 32x32 circle when only SVG is shown
  capsulePaddingLeftConcentric: 1, // concentric 4px bezel (1px padding + 3px SVG viewBox margin)
  capsulePaddingRight: 12, // breathing room after lyric text or prompt buttons
  capsulePaddingX: 10, // symmetric padding when leftContent is also present
  contentGap: 6,
};

export const EXPANDED_BASE = {
  minWidth: 300,
  height: 128,
  avatarSize: 108,
  paddingX: 24,
  radius: 42, // Classic iOS expanded Dynamic Island pill/capsule corner radius
  contentGap: 12,
};

export const TRANSLATING_METRICS: IslandMetrics = {
  width: 420,
  height: 252,
  radius: 34,
  avatarSize: 64,
};

export const MUSIC_EXPANDED_METRICS: IslandMetrics = {
  width: 396,
  height: 174,
  radius: 40,
  avatarSize: 64,
};

/**
 * Slightly enlarged "hover peek" dimensions shown immediately when the mouse enters
 * the collapsed island while waiting for the hover dwell delay before full expansion.
 */
export function computePeekMetrics(
  collapsed: IslandMetrics,
  maxWidth: number = ISLAND_HOST_WIDTH - 32,
): IslandMetrics {
  const isCircle = Math.abs(collapsed.width - collapsed.height) < 1;
  const height = collapsed.height + 6; // 32 -> 38
  const width = isCircle ? height : Math.min(collapsed.width + 8, maxWidth);
  return {
    width,
    height,
    radius: height / 2,
    avatarSize: collapsed.avatarSize + 5, // 30 -> 35
  };
}

export function computeIslandMetrics(
  expanded: boolean,
  extraWidth: number,
  activeSlots: number,
  maxWidth: number = ISLAND_HOST_WIDTH - 32,
  rightOnly: boolean = false,
): IslandMetrics {
  const hasExtra = extraWidth > 0.5 && activeSlots > 0;

  if (!expanded) {
    const {
      height,
      avatarSize,
      circlePaddingX,
      capsulePaddingLeftConcentric,
      capsulePaddingRight,
      capsulePaddingX,
      contentGap,
    } = COLLAPSED_BASE;
    const rawWidth = hasExtra
      ? rightOnly
        ? capsulePaddingLeftConcentric +
          avatarSize +
          contentGap +
          extraWidth +
          capsulePaddingRight
        : avatarSize + extraWidth + activeSlots * contentGap + capsulePaddingX * 2
      : avatarSize + circlePaddingX * 2;
    const width = Math.min(Math.max(height, Math.ceil(rawWidth)), maxWidth);

    return {
      width,
      height,
      radius: height / 2, // Circle when width == height; full pill/capsule when width > height
      avatarSize,
    };
  }

  // Expanded state always takes the wide iOS Dynamic Island capsule form,
  // expanding even wider automatically if extra content exceeds minWidth.
  const { minWidth, height, avatarSize, paddingX, radius, contentGap } = EXPANDED_BASE;
  const contentWidth = hasExtra
    ? avatarSize + extraWidth + activeSlots * contentGap + paddingX * 2
    : minWidth;
  const width = Math.min(Math.max(minWidth, Math.ceil(contentWidth)), maxWidth);

  return {
    width,
    height,
    radius,
    avatarSize,
  };
}

export const DEFAULT_COLLAPSED_METRICS: IslandMetrics = computeIslandMetrics(false, 0, 0);
export const DEFAULT_EXPANDED_METRICS: IslandMetrics = computeIslandMetrics(true, 0, 0);

// Underdamped spring presets (damping ratio ~0.50-0.53) tuned for tactile iOS jelly bounce.
// Height & avatarSize slightly trail width so the island stretches & blooms organically.
const EXPAND_SPRINGS = {
  width: { stiffness: 350, damping: 19.5, mass: 1.0 },
  height: { stiffness: 270, damping: 16.5, mass: 1.02 },
  radius: { stiffness: 300, damping: 19.5, mass: 1.0 },
  avatarSize: { stiffness: 280, damping: 17.0, mass: 1.02 },
};

const COLLAPSE_SPRINGS = {
  width: { stiffness: 380, damping: 20.5, mass: 1.0 },
  height: { stiffness: 320, damping: 18.0, mass: 1.0 },
  radius: { stiffness: 340, damping: 20.5, mass: 1.0 },
  avatarSize: { stiffness: 330, damping: 18.5, mass: 1.0 },
};

// Underdamped pulse springs (damping ratio ~0.39) anchored at 0 for crisp iOS click-puff bounce.
// Slight stiffness stagger between X and Y gives a lively organic jelly swell and rebound.
const PULSE_SPRINGS = {
  x: { stiffness: 440, damping: 16.5, mass: 1.0 },
  y: { stiffness: 360, damping: 15.0, mass: 1.04 },
};

function clamp(v: number, min: number, max: number): number {
  return Math.min(Math.max(v, min), max);
}

export class JellyIslandPhysics {
  private width: SpringAxis;
  private height: SpringAxis;
  private radius: SpringAxis;
  private avatarSize: SpringAxis;
  private pulseX: SpringAxis;
  private pulseY: SpringAxis;

  constructor(initial: IslandMetrics = DEFAULT_COLLAPSED_METRICS) {
    this.width = new SpringAxis(initial.width, COLLAPSE_SPRINGS.width, 0.05);
    this.height = new SpringAxis(initial.height, COLLAPSE_SPRINGS.height, 0.05);
    this.radius = new SpringAxis(initial.radius, COLLAPSE_SPRINGS.radius, 0.05);
    this.avatarSize = new SpringAxis(initial.avatarSize, COLLAPSE_SPRINGS.avatarSize, 0.05);
    this.pulseX = new SpringAxis(0, PULSE_SPRINGS.x, 0.03);
    this.pulseY = new SpringAxis(0, PULSE_SPRINGS.y, 0.03);
  }

  public setTarget(metrics: IslandMetrics, expanding: boolean): void {
    const springs = expanding ? EXPAND_SPRINGS : COLLAPSE_SPRINGS;

    this.width.setTarget(metrics.width, springs.width);
    this.height.setTarget(metrics.height, springs.height);
    this.radius.setTarget(metrics.radius, springs.radius);
    this.avatarSize.setTarget(metrics.avatarSize, springs.avatarSize);
  }

  /**
   * Injects a velocity impulse into the pulse springs so the collapsed island
   * swells slightly (~9px width / ~8px height) and rebounds with tactile iOS jelly physics.
   */
  public triggerClickPulse(): void {
    this.pulseX.addImpulse(310, 420);
    this.pulseY.addImpulse(245, 340);
  }

  public getCurrentAvatarSize(): number {
    return Math.max(16, this.avatarSize.value + this.pulseY.value * 0.72);
  }

  public step(dt: number): IslandFrame {
    this.width.step(dt);
    this.height.step(dt);
    this.radius.step(dt);
    this.avatarSize.step(dt);
    this.pulseX.step(dt);
    this.pulseY.step(dt);

    const totalVw = this.width.velocity + this.pulseX.velocity * 1.4;
    const totalVh = this.height.velocity + this.pulseY.velocity * 1.4;

    // Volume-conserving jelly squash & stretch driven by real-time spring velocities.
    // Smoothly decays to exact identity (1, 1, 0) as the springs settle.
    const normVw = clamp(totalVw / 2400, -1, 1);
    const normVh = clamp(totalVh / 1100, -1, 1);

    const scaleX = clamp(1 + normVw * 0.038 - normVh * 0.024, 0.95, 1.05);
    const scaleY = clamp(1 + normVh * 0.044 - normVw * 0.02, 0.94, 1.06);
    const translateY = clamp(normVh * 3.2 + this.pulseY.value * 0.14, -3.2, 4.5);

    const settled =
      this.width.isAtRest() &&
      this.height.isAtRest() &&
      this.radius.isAtRest() &&
      this.avatarSize.isAtRest() &&
      this.pulseX.isAtRest() &&
      this.pulseY.isAtRest();

    const width = Math.max(20, this.width.value + this.pulseX.value);
    const height = Math.max(20, this.height.value + this.pulseY.value);
    const radius = Math.max(10, Math.min(this.radius.value + this.pulseY.value * 0.5, height / 2));
    const avatarSize = Math.max(16, this.avatarSize.value + this.pulseY.value * 0.72);

    return {
      width,
      height,
      radius,
      avatarSize,
      scaleX: settled ? 1 : scaleX,
      scaleY: settled ? 1 : scaleY,
      translateY: settled ? 0 : translateY,
      settled,
    };
  }
}
