import { chromium } from './node_modules/playwright/index.mjs';

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage();

page.on('console', msg => console.log(`[console] ${msg.type()} ${msg.text()}`));
page.on('pageerror', err => console.log(`[pageerror] ${err.message}`));

await page.goto('http://localhost:5173/');
await page.waitForLoadState('networkidle');

const title = await page.title();
console.log('Page title:', title);

// Check if we need to log in
const text = await page.textContent('body');
if (text.includes('Sign in') || text.includes('Login')) {
  console.log('Login page detected');
} else {
  console.log('Already logged in or no login needed');
  console.log('Body excerpt:', text.substring(0, 200));
}

await browser.close();
