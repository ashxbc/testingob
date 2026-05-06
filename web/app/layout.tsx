import type { Metadata, Viewport } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'huntr · BTC liquidation hunter',
  description:
    'Real-time spot orderbook walls + cross-exchange liquidation clusters. Where price is being steered.',
  appleWebApp: {
    capable: true,
    statusBarStyle: 'black-translucent',
    title: 'huntr',
  },
};

export const viewport: Viewport = {
  width: 'device-width',
  initialScale: 1,
  maximumScale: 1,
  themeColor: [
    { media: '(prefers-color-scheme: light)', color: '#ffffff' },
    { media: '(prefers-color-scheme: dark)', color: '#000000' },
  ],
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
