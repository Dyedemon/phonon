import { test } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';
import { installStub } from './_tmp-stub';

const pluginSrc = fs.readFileSync(path.resolve(process.cwd(), '..', 'plugins', 'vis', 'vis_mode.js'), 'utf8');
const SHOTS = 14;

test('nebula temporal flicker probe', async ({ page, context }) => {
  await context.addInitScript(installStub, pluginSrc);
  await page.setViewportSize({ width: 960, height: 640 });
  page.on('pageerror', (e) => console.log('[err]', String(e).slice(0, 150)));
  await page.goto('/', { waitUntil: 'domcontentloaded', timeout: 30_000 });
  await page.waitForFunction(() => !!document.getElementById('root')?.firstElementChild, undefined, { timeout: 40_000 });
  const visBtn = page.locator('button[aria-label="视觉升华设置"]');
  await visBtn.waitFor({ state: 'visible', timeout: 15_000 });
  await visBtn.click();
  await page.waitForTimeout(300);
  await page.getByText('三维承空', { exact: true }).first().click();
  await page.waitForTimeout(300);
  await page.locator('.vis-mode-toggle').click();
  await page.waitForTimeout(500);
  await page.getByRole('button', { name: '进入维度' }).click();
  await page.waitForTimeout(8_000); // 等淡入和场景稳定

  // 连续截图
  const dir = 'D:/phonon-flicker';
  fs.mkdirSync(dir, { recursive: true });
  for (let i = 0; i < SHOTS; i++) {
    await page.screenshot({ path: `${dir}/f${String(i).padStart(2, '0')}.png` });
    await page.waitForTimeout(200);
  }
  console.log('[diag] captured', SHOTS, 'frames');
});
