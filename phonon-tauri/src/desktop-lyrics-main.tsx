import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import DesktopLyrics from './components/DesktopLyrics'

// Dedicated entry point for the desktop lyrics floating window.
// This file is loaded ONLY by desktop-lyrics.html — there is no window
// detection logic here, so it is impossible for this code to run in the
// main window (or vice versa). This completely eliminates the class of
// bugs where the floating window accidentally renders <App /> and its
// useEffects conflict with the main window.

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <DesktopLyrics />
  </StrictMode>,
)
