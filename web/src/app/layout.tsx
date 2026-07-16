import type { Metadata, Viewport } from "next";
import "@fontsource-variable/dm-sans";
import "@fontsource-variable/outfit";
import "@fontsource-variable/jetbrains-mono";
import "./globals.css";

export const metadata: Metadata = {
  title: "Catalyst Code",
  description: "A web interface for the Catalyst Code harness.",
  icons: {
    icon:
      "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><rect width='100' height='100' rx='22' fill='%230a0a0a'/><text x='50' y='68' font-size='58' text-anchor='middle' font-family='monospace' fill='%23cf8a59'>c</text></svg>",
  },
};

export const viewport: Viewport = {
  themeColor: "#0a0a0a",
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script
          dangerouslySetInnerHTML={{
            __html: `try{var t=localStorage.getItem('catalyst:theme')||localStorage.getItem('umans:theme')||'dark';if(t!=='dark'&&t!=='light')t='dark';document.documentElement.setAttribute('data-theme',t);try{localStorage.setItem('catalyst:theme',t)}catch(e2){}}catch(e){}`,
          }}
        />
      </head>
      <body className="font-sans antialiased">{children}</body>
    </html>
  );
}
