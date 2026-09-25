import type { ReactNode, SVGProps } from 'react';

// UnknownChat icon set: 16px grid, 1.5px stroke, round caps, currentColor.
// Decorative by default; give the owning control an accessible name.
type IconProps = Omit<SVGProps<SVGSVGElement>, 'children'> & { size?: number };

function Icon({ size = 16, children, ...rest }: IconProps & { children: ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor"
      strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false" {...rest}>
      {children}
    </svg>
  );
}

export const ChannelIcon = (p: IconProps) => <Icon {...p}><path d="M6.2 2.5 5 13.5M11 2.5 9.8 13.5M2.8 5.8h10.9M2.3 10.2h10.9" /></Icon>;
export const SearchIcon = (p: IconProps) => <Icon {...p}><circle cx="7" cy="7" r="4.3" /><path d="m10.3 10.3 3.2 3.2" /></Icon>;
export const ChatIcon = (p: IconProps) => <Icon {...p}><path d="M2.5 3.5h11V11H8.2L5 13.5V11H2.5z" /><path d="M5.5 6.3h5M5.5 8.5h3.2" /></Icon>;
export const LockIcon = (p: IconProps) => <Icon {...p}><rect x="3.5" y="7" width="9" height="6.5" rx="1.5" /><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" /></Icon>;
export const OpenLockIcon = (p: IconProps) => <Icon {...p}><rect x="3.5" y="7" width="9" height="6.5" rx="1.5" /><path d="M5.5 7V5a2.5 2.5 0 0 1 4.8-1" /></Icon>;
export const KeyIcon = (p: IconProps) => <Icon {...p}><circle cx="5.2" cy="10.8" r="2.7" /><path d="M7.1 8.9 13.2 2.8M11.2 4.8l1.6 1.6M9.6 6.4l1.2 1.2" /></Icon>;
export const PanelIcon = (p: IconProps) => <Icon {...p}><rect x="2" y="3" width="12" height="10" rx="1.5" /><path d="M10 3v10" /></Icon>;
export const BackIcon = (p: IconProps) => <Icon {...p}><path d="M13 8H3M7 4 3 8l4 4" /></Icon>;
export const SendIcon = (p: IconProps) => <Icon strokeWidth={1.75} {...p}><path d="M8 13V3.5M4 7.5l4-4 4 4" /></Icon>;
export const PlusIcon = (p: IconProps) => <Icon {...p}><path d="M8 3v10M3 8h10" /></Icon>;
export const LogoutIcon = (p: IconProps) => <Icon {...p}><path d="M6.5 13.5h-3v-11h3M10 11l3-3-3-3M13 8H6" /></Icon>;
export const PeopleIcon = (p: IconProps) => <Icon {...p}><circle cx="6" cy="5.5" r="2.3" /><path d="M2 13c.4-2.3 2-3.6 4-3.6s3.6 1.3 4 3.6" /><path d="M10.5 3.4a2.2 2.2 0 0 1 0 4.2M11.8 9.6c1.2.4 2 1.6 2.2 3.4" /></Icon>;
export const StatsIcon = (p: IconProps) => <Icon {...p}><path d="M2.5 13.5h11M4.5 11V8M8 11V4.5M11.5 11V6.5" /></Icon>;
export const LinkIcon = (p: IconProps) => <Icon {...p}><path d="M6.8 9.2a2.6 2.6 0 0 0 3.7 0l2.2-2.2a2.6 2.6 0 0 0-3.7-3.7l-.9.9" /><path d="M9.2 6.8a2.6 2.6 0 0 0-3.7 0L3.3 9a2.6 2.6 0 0 0 3.7 3.7l.9-.9" /></Icon>;
export const EyeIcon = ({ crossed = false, ...p }: IconProps & { crossed?: boolean }) => (
  <Icon {...p}><path d="M1.8 8S4 3.8 8 3.8 14.2 8 14.2 8 12 12.2 8 12.2 1.8 8 1.8 8Z" /><circle cx="8" cy="8" r="2" />{crossed && <path d="m2.8 2.3 10.4 11.4" />}</Icon>
);
export const ArrowIcon = (p: IconProps) => <Icon {...p}><path d="M3 8h10M9 4l4 4-4 4" /></Icon>;
export const DevicesIcon = (p: IconProps) => <Icon {...p}><rect x="1.8" y="3" width="8.7" height="7" rx="1.2" /><path d="M4.5 12.8h3.6M6.2 10v2.8" /><rect x="11.2" y="6.2" width="3" height="6.6" rx=".8" /></Icon>;

/** Brand mark: the cut-corner frame with an eclipsed moon. */
export function BrandMark({ size = 24, className }: { size?: number; className?: string }) {
  return (
    <svg className={className} width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true" focusable="false">
      <path d="M6.5 2H22V17.5L17.5 22H2V6.5Z" stroke="var(--uv-500)" strokeWidth="1.5" strokeLinejoin="round" />
      <circle cx="12" cy="12" r="5.5" fill="var(--uv-300)" />
      <circle cx="13.9" cy="10.4" r="4.9" fill="var(--bg-chassis)" />
    </svg>
  );
}
