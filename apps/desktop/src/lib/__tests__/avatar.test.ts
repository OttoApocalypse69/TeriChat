import { describe, expect, it } from 'vitest';
import { AVATAR_GRADIENTS, avatarGradient } from '../avatar';

describe('avatarGradient', () => {
  it('is stable for the same id', () => {
    expect(avatarGradient('user-7f3a')).toBe(avatarGradient('user-7f3a'));
  });

  it('always picks from the design palette', () => {
    const palette = AVATAR_GRADIENTS.map(([from, to]) => `linear-gradient(135deg, ${from}, ${to})`);
    for (const id of ['', 'a', 'b', 'peer', '00000000-0000-0000-0000-000000000000', 'ÿ-unicode-✓']) {
      expect(palette).toContain(avatarGradient(id));
    }
  });

  it('spreads distinct ids across more than one colour', () => {
    const seen = new Set(Array.from({ length: 32 }, (_, i) => avatarGradient(`synthetic-user-${i}`)));
    expect(seen.size).toBeGreaterThan(1);
  });
});
