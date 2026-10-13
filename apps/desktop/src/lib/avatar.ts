// Person avatars are circles filled with a gradient until an image exists.
// The gradient is derived from a stable id so a person keeps one colour
// across lists, headers and messages without any server-side preference.
export const AVATAR_GRADIENTS = [
  ['#8B63FF', '#3A2296'],
  ['#4C9BFF', '#1C3F94'],
  ['#3DD6C4', '#135E68'],
  ['#FF6B85', '#7E1E3E'],
  ['#F7C25E', '#A4561A'],
  ['#D08BFF', '#5B2A8E'],
  ['#FF8ACB', '#8E2A66'],
  ['#FF9A5C', '#8E3514'],
] as const;

/** FNV-1a over UTF-16 code units: small, stable and dependency-free. */
function hash(key: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < key.length; i += 1) {
    h ^= key.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

export function avatarGradient(key: string): string {
  const [from, to] = AVATAR_GRADIENTS[hash(key) % AVATAR_GRADIENTS.length];
  return `linear-gradient(135deg, ${from}, ${to})`;
}
