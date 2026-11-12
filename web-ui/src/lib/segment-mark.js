export const PATH_LENGTH = 114;
export const SEGMENT_OFFSETS = [0, -38, -76];
export const REST_DASH = '30 84';

const STEP_DEGREES = 120;
const START_SLOPE = 0.6 / 0.25;
const MIN_SECONDS = 0.6;
const MAX_SECONDS = 2.4;
const MIN_TRAVEL = 25;
const MATRIX_RE = /^matrix\(\s*([-\d.e]+)\s*,\s*([-\d.e]+)/;

export function angleOf(transform) {
  const match = MATRIX_RE.exec(transform ?? '');
  if (!match) return 0;
  return (Math.atan2(Number(match[2]), Number(match[1])) * 180) / Math.PI;
}

export function forwardDelta(from, to) {
  return (((to - from) % 360) + 360) % 360;
}

export function settlePlan(angle, speed) {
  const stoppingDistance = Math.max(MIN_TRAVEL, (speed * MIN_SECONDS) / START_SLOPE);
  const target = Math.ceil((angle + stoppingDistance) / STEP_DEGREES) * STEP_DEGREES;
  const travel = target - angle;
  const seconds = Math.min(MAX_SECONDS, Math.max(MIN_SECONDS, (START_SLOPE * travel) / Math.max(speed, 1)));
  return { target, seconds };
}
