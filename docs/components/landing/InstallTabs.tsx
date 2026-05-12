'use client';

import { Check, Copy, ExternalLink } from 'lucide-react';
import { useState } from 'react';

type InstallTabsProps = {
  lang: 'en' | 'cn';
};

const releaseUrl = 'https://github.com/t8y2/dbx/releases/latest';

const installOptions = {
  en: [
    {
      id: 'macos',
      label: 'macOS',
      note: 'Homebrew',
      command: 'brew install --cask t8y2/tap/dbx',
      linkText: 'GitHub Releases',
    },
    {
      id: 'windows',
      label: 'Windows',
      note: 'Scoop',
      command: 'scoop bucket add dbx https://github.com/t8y2/scoop-bucket; scoop install dbx',
      linkText: 'GitHub Releases',
    },
    {
      id: 'linux',
      label: 'Linux',
      note: '.deb / AppImage',
      command: 'chmod +x DBX*.AppImage',
      linkText: 'Download packages',
    },
    {
      id: 'docker',
      label: 'Docker',
      note: 'Self-hosted',
      command: 'docker run -d --name dbx -p 4224:4224 -v dbx-data:/app/data t8y2/dbx',
      linkText: 'Docker docs',
      href: '/en/docs/getting-started#docker',
    },
  ],
  cn: [
    {
      id: 'macos',
      label: 'macOS',
      note: 'Homebrew',
      command: 'brew install --cask t8y2/tap/dbx',
      linkText: 'GitHub Releases',
    },
    {
      id: 'windows',
      label: 'Windows',
      note: 'Scoop',
      command: 'scoop bucket add dbx https://github.com/t8y2/scoop-bucket; scoop install dbx',
      linkText: 'GitHub Releases',
    },
    {
      id: 'linux',
      label: 'Linux',
      note: '.deb / AppImage',
      command: 'chmod +x DBX*.AppImage',
      linkText: '下载安装包',
    },
    {
      id: 'docker',
      label: 'Docker',
      note: '自托管',
      command: 'docker run -d --name dbx -p 4224:4224 -v dbx-data:/app/data t8y2/dbx',
      linkText: 'Docker 文档',
      href: '/cn/docs/getting-started#docker',
    },
  ],
};

export function InstallTabs({ lang }: InstallTabsProps) {
  const options = installOptions[lang];
  const [activeId, setActiveId] = useState(options[0].id);
  const [copied, setCopied] = useState(false);
  const active = options.find((item) => item.id === activeId) ?? options[0];

  async function copyCommand() {
    await navigator.clipboard.writeText(active.command);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  }

  return (
    <div className="landing-install">
      <div className="landing-install-tabs" role="tablist" aria-label={lang === 'cn' ? '安装平台' : 'Install platform'}>
        {options.map((item) => (
          <button
            aria-selected={item.id === active.id}
            className="landing-install-tab"
            key={item.id}
            onClick={() => {
              setActiveId(item.id);
              setCopied(false);
            }}
            role="tab"
            type="button"
          >
            <span>{item.label}</span>
            <small>{item.note}</small>
          </button>
        ))}
      </div>
      <div className="landing-install-command">
        <code>{active.command}</code>
        <button
          aria-label={lang === 'cn' ? '复制安装命令' : 'Copy install command'}
          className="landing-install-copy"
          onClick={copyCommand}
          title={lang === 'cn' ? '复制安装命令' : 'Copy install command'}
          type="button"
        >
          {copied ? <Check size={16} /> : <Copy size={16} />}
        </button>
      </div>
      <a className="landing-install-release" href={active.href ?? releaseUrl} target="_blank">
        {active.linkText}
        <ExternalLink size={14} />
      </a>
    </div>
  );
}
