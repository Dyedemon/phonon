import sharp from 'sharp'
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'fs'
import { join, dirname } from 'path'
import { fileURLToPath } from 'url'

const __dirname = dirname(fileURLToPath(import.meta.url))

const svgPath = join(__dirname, '..', 'public', 'phonon-icon.svg')
const iconsDir = join(__dirname, '..', 'src-tauri', 'icons')

if (!existsSync(iconsDir)) {
  mkdirSync(iconsDir, { recursive: true })
}

const svgBuffer = readFileSync(svgPath)

const sizes = [
  { size: 32, name: '32x32.png' },
  { size: 128, name: '128x128.png' },
  { size: 256, name: '128x128@2x.png' },
  { size: 512, name: 'icon.png' },
  { size: 50, name: 'StoreLogo.png' },
  { size: 30, name: 'Square30x30Logo.png' },
  { size: 44, name: 'Square44x44Logo.png' },
  { size: 71, name: 'Square71x71Logo.png' },
  { size: 89, name: 'Square89x89Logo.png' },
  { size: 107, name: 'Square107x107Logo.png' },
  { size: 142, name: 'Square142x142Logo.png' },
  { size: 150, name: 'Square150x150Logo.png' },
  { size: 284, name: 'Square284x284Logo.png' },
  { size: 310, name: 'Square310x310Logo.png' },
]

console.log('Generating icon sizes...')

for (const { size, name } of sizes) {
  const outPath = join(iconsDir, name)
  await sharp(svgBuffer)
    .resize(size, size, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png()
    .toFile(outPath)
  console.log(`  ✓ ${name} (${size}x${size})`)
}

console.log('\nAll PNG icons generated!')
console.log('\nNext: run `npx tauri icon src-tauri/icons/icon.png` to generate ICO/ICNS')
