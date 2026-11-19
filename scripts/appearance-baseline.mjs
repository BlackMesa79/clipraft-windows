// 在既有真实渲染节点记录计算样式；比较基线可选，不注入产品CSS。
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const selectors = [
  '.main-window',
  '.main-window > header',
  '.group-sidebar',
  '.searchbox',
  '.kind-filters button',
  '.item',
  '.item.selected',
  '.summary',
  '.type-icon',
  '.record-thumbnail',
  '.detail',
  '.detail-bottom',
  '.paste-primary',
  '.preview-secondary button',
  '.context-menu',
  '.context-menu button',
  '.settings-nav',
  '.settings-content',
  '.settings-actions',
  '.number-field input',
  '.settings-buttons button',
  '.settings-view button:disabled',
  '.check input',
  '.error',
  '.sensitive-badge',
  '.file-status',
  '.group-badge',
  '.archive-dialog',
  '.archive-dialog input',
  '.archive-dialog button',
  '.staging-cleanup-dialog',
  '.directory-confirm',
  '.rich-text',
  '.rich-picture img',
  '.floating-preview',
  '.floating-text',
  '.floating-content',
  '.wheel-card.selected',
  '.quick .count',
  '.quick .wheel-capsule',
];
const properties = [
  'color',
  'backgroundColor',
  'borderColor',
  'borderWidth',
  'borderRadius',
  'fontFamily',
  'fontSize',
  'fontWeight',
  'lineHeight',
  'boxShadow',
  'opacity',
  'outlineWidth',
  'outlineOffset',
  'outlineStyle',
  'padding',
  'gap',
  'width',
  'height',
];

/**
 * @param {(expression: string) => Promise<unknown>} evaluate
 * @param {string} outputDirectory
 */
export function createAppearanceRecorder(evaluate, outputDirectory) {
  /** @type {Record<string, unknown>} */
  const snapshots = {};
  return {
    /** @param {string} name */
    async capture(name) {
      snapshots[name] = await evaluate(`(() => {
        const snapshot = {};
        for (const selector of ${JSON.stringify(selectors)}) {
          const element = document.querySelector(selector);
          if (!element || !element.getClientRects().length) continue;
          const style = getComputedStyle(element);
          snapshot[selector] = Object.fromEntries(
            ${JSON.stringify(properties)}.map(key => [key, style[key]])
          );
        }
        return snapshot;
      })()`);
    },
    async finish() {
      await writeFile(
        path.join(outputDirectory, 'appearance.json'),
        JSON.stringify(snapshots, null, 2),
      );
      if (!process.env.CLIPRAFT_APPEARANCE_BASELINE) return;
      const filename = path.join(
        process.env.CLIPRAFT_APPEARANCE_BASELINE,
        path.basename(outputDirectory) + '.json',
      );
      const expected = JSON.parse(await readFile(filename, 'utf8'));
      for (const [name, snapshot] of Object.entries(snapshots)) {
        assert.deepEqual(snapshot, expected[name], `外观计算样式变化：${name}`);
      }
      assert.deepEqual(Object.keys(snapshots), Object.keys(expected), '外观基线覆盖范围变化');
    },
  };
}
