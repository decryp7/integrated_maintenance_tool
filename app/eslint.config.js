// @ts-check
// Flat ESLint config for the IMT Angular UI: ESLint + typescript-eslint recommended rules and
// angular-eslint recommended rules for TypeScript and templates (including accessibility).
const eslint = require('@eslint/js');
const { defineConfig } = require('eslint/config');
const tseslint = require('typescript-eslint');
const angular = require('angular-eslint');

module.exports = defineConfig(
  {
    ignores: [
      'dist/**',
      '.angular/**',
      'coverage/**',
      'node_modules/**',
      'src-tauri/**',
      'test-results/**',
      'playwright-report/**',
    ],
  },
  {
    files: ['**/*.ts'],
    extends: [
      eslint.configs.recommended,
      tseslint.configs.recommended,
      angular.configs.tsRecommended,
    ],
    processor: angular.processInlineTemplates,
    rules: {
      '@angular-eslint/directive-selector': [
        'error',
        { type: 'attribute', prefix: 'imt', style: 'camelCase' },
      ],
      '@angular-eslint/component-selector': [
        'error',
        { type: 'element', prefix: 'imt', style: 'kebab-case' },
      ],
      // The codebase marks intentionally unused parameters/bindings (interface-shaped stubs, rest-omission
      // destructuring such as `const { dropped: _d, ...rest } = x`) with a leading underscore.
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrorsIgnorePattern: '^_',
          destructuredArrayIgnorePattern: '^_',
          ignoreRestSiblings: true,
        },
      ],
      // `let ref; const done = () => ref?.destroy(); ref = effect(...)`: a closure that reads the binding before
      // its single assignment cannot use `const` (TDZ), so those are not reported.
      'prefer-const': ['error', { ignoreReadBeforeAssign: true }],
    },
  },
  {
    files: ['**/*.html'],
    extends: [angular.configs.templateRecommended, angular.configs.templateAccessibility],
    rules: {
      // IMT's composite widgets (combobox listboxes, grids, tree grids, trees, tab lists, menus) follow the
      // WAI-ARIA aria-activedescendant / roving-tabindex patterns: focus and keyboard handling live on the
      // owning input or container (or on a [attr.tabindex]-bound child these rules cannot see), and items are
      // deliberately not tab stops. These two rules only reported such cases, so they are off.
      '@angular-eslint/template/click-events-have-key-events': 'off',
      '@angular-eslint/template/interactive-supports-focus': 'off',
    },
  },
);
