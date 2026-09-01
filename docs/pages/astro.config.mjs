import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://takeshid.github.io',
  base: '/pcx',
  integrations: [
    starlight({
      title: 'pcx',
      description: 'Inspect and reduce point-cloud recordings where the data lives.',
      locales: {
        root: { label: 'English', lang: 'en' },
        ja: { label: '日本語', lang: 'ja' },
      },
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/takeshiD/pcx',
        },
      ],
      editLink: {
        baseUrl: 'https://github.com/takeshiD/pcx/edit/main/docs/pages/',
      },
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        {
          label: 'Start here',
          translations: { ja: 'はじめに' },
          items: ['installation', 'quick-start', 'status'],
        },
        {
          label: 'Design',
          translations: { ja: '設計' },
          items: ['architecture', 'formats', 'commands'],
        },
        {
          label: 'Project',
          translations: { ja: 'プロジェクト' },
          items: ['testing', 'contributing', 'release', 'security', 'decisions'],
        },
      ],
    }),
  ],
});
