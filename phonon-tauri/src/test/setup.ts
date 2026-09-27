/**
 * Vitest global setup — runs before every test file.
 *
 * Provides a clean localStorage for each test (jsdom persists it across
 * files otherwise) and registers the @testing-library/jest-dom matchers.
 */

import '@testing-library/jest-dom';
import { beforeEach } from 'vitest';

// Reset localStorage between tests so storage spec tests start clean.
beforeEach(() => {
  localStorage.clear();
});
