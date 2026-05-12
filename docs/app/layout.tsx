import './global.css';
import type { ReactNode } from 'react';

export const metadata = {
  title: {
    default: 'DBX',
    template: '%s | DBX',
  },
  description: '25+ databases in 15 MB. Desktop & Docker self-hosting, with built-in AI assistant.',
  icons: {
    icon: '/favicon.png',
    shortcut: '/favicon.png',
    apple: '/logo.png',
  },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return children;
}
