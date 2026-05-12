import Link from 'next/link';
import type { CSSProperties } from 'react';
import { HeroProductStage } from '@/components/aceternity/HeroProductStage';
import { InfiniteMovingCards } from '@/components/aceternity/InfiniteMovingCards';
import { Spotlight } from '@/components/aceternity/Spotlight';
import { InstallTabs } from '@/components/landing/InstallTabs';
import { RevealSection } from '@/components/landing/RevealSection';
import {
  ArrowRight,
  Bot,
  CheckCircle2,
  Database,
  FileCode,
  GitCompare,
  Network,
  Search,
  Shield,
  Table,
  Terminal,
  Zap,
} from 'lucide-react';

const fallbackStarLabel = '1.3k+';

function formatStars(count: number) {
  if (count >= 1000) {
    return `${(Math.floor(count / 100) / 10).toFixed(1)}k+`;
  }

  return `${count}+`;
}

async function getGitHubStarLabel() {
  try {
    const response = await fetch('https://api.github.com/repos/t8y2/dbx', {
      headers: { Accept: 'application/vnd.github+json' },
      next: { revalidate: 60 * 60 * 6 },
    });

    if (!response.ok) return fallbackStarLabel;

    const data = (await response.json()) as { stargazers_count?: number };
    return typeof data.stargazers_count === 'number' ? formatStars(data.stargazers_count) : fallbackStarLabel;
  } catch {
    return fallbackStarLabel;
  }
}

function metrics(starLabel: string) {
  return {
    en: [
    { value: '~15 MB', label: 'desktop installer' },
    { value: '25+', label: 'database engines' },
    { value: '2 modes', label: 'desktop and Docker' },
      { value: starLabel, label: 'GitHub stars, fully open-source' },
    ],
    cn: [
    { value: '~15 MB', label: '桌面安装包' },
    { value: '25+', label: '数据库引擎' },
    { value: '2 种模式', label: '桌面与 Docker' },
      { value: starLabel, label: 'GitHub Star，完全开源' },
    ],
  };
}

const databaseSupport = [
  { name: 'MySQL', icon: '/icons/database/mysql.svg', tone: '#4479a1' },
  { name: 'PostgreSQL', icon: '/icons/database/postgres.svg', tone: '#4169e1' },
  { name: 'SQLite', icon: '/icons/database/sqlite.svg', tone: '#5aa6d6' },
  { name: 'Redis', icon: '/icons/database/redis.svg', tone: '#ff4438' },
  { name: 'DuckDB', icon: '/icons/database/duckdb.svg', tone: '#fff000' },
  { name: 'ClickHouse', icon: '/icons/database/clickhouse.svg', tone: '#ffcc01' },
  { name: 'SQL Server', icon: '/icons/database/sqlserver.svg', tone: '#9ca3af' },
  { name: 'MongoDB', icon: '/icons/database/mongodb.svg', tone: '#47a248' },
  { name: 'Oracle', icon: '/icons/database/oracle.svg', tone: '#f80000' },
  { name: 'Elasticsearch', icon: '/icons/database/elasticsearch.svg', tone: '#00bfb3' },
  { name: 'Doris', icon: '/icons/database/doris.svg', tone: '#5b7cfa' },
  { name: 'StarRocks', icon: '/icons/database/starrocks.svg', tone: '#6750ff' },
  { name: 'Redshift', icon: '/icons/database/redshift.svg', tone: '#8c4fff' },
  { name: 'Dameng', icon: '/icons/database/dm.svg', tone: '#3857ff' },
  { name: 'GaussDB', icon: '/icons/database/gaussdb.svg', tone: '#ff5a3d' },
  { name: 'JDBC', initials: 'JDBC', tone: '#6ea8ff' },
  { name: 'TiDB', icon: '/icons/database/tidb.svg', tone: '#e60012' },
  { name: 'OceanBase', icon: '/icons/database/oceanbase.svg', tone: '#2285ff' },
  { name: 'SelectDB', icon: '/icons/database/selectdb.svg', tone: '#22c1c3' },
  { name: 'TDengine', icon: '/icons/database/tdengine.svg', tone: '#2f6fff' },
  { name: 'openGauss', icon: '/icons/database/opengauss.svg', tone: '#1488c9' },
  { name: 'KingBase', icon: '/icons/database/kingbase.svg', tone: '#e1212d' },
  { name: 'CockroachDB', icon: '/icons/database/cockroachdb.svg', tone: '#6933ff' },
  { name: 'More', initials: '···', tone: '#6ea8ff' },
];

const workflows = {
  en: [
    {
      icon: Terminal,
      title: 'Write and run SQL',
      desc: 'A CodeMirror 6 editor with metadata-aware completion, formatting, history, and selected SQL execution.',
      href: '/en/docs/query-editor',
    },
    {
      icon: Table,
      title: 'Browse and edit data',
      desc: 'Virtualized grids, inline editing, WHERE/ORDER BY controls, SQL preview, and export tools.',
      href: '/en/docs/data-grid',
    },
    {
      icon: Search,
      title: 'Explore schemas',
      desc: 'Navigate databases, schemas, tables, columns, indexes, foreign keys, and triggers from a focused sidebar.',
      href: '/en/docs/schema-browser',
    },
    {
      icon: GitCompare,
      title: 'Compare and migrate',
      desc: 'Schema diff, table import, database export, SQL file execution, and cross-engine data transfer.',
      href: '/en/docs/schema-diff',
    },
  ],
  cn: [
    {
      icon: Terminal,
      title: '编写与执行 SQL',
      desc: 'CodeMirror 6 编辑器，支持元数据补全、格式化、查询历史和选中 SQL 执行。',
      href: '/cn/docs/query-editor',
    },
    {
      icon: Table,
      title: '浏览与编辑数据',
      desc: '虚拟滚动表格、行内编辑、WHERE/ORDER BY 控制、SQL 预览和导出工具。',
      href: '/cn/docs/data-grid',
    },
    {
      icon: Search,
      title: '浏览数据库结构',
      desc: '在侧边栏中查看数据库、Schema、表、字段、索引、外键和触发器。',
      href: '/cn/docs/schema-browser',
    },
    {
      icon: GitCompare,
      title: '对比与迁移',
      desc: 'Schema 对比、表导入、数据库导出、SQL 文件执行和跨引擎数据传输。',
      href: '/cn/docs/schema-diff',
    },
  ],
};

const capabilities = {
  en: [
    { icon: Database, label: 'Native Rust drivers, no JDBC runtime' },
    { icon: Shield, label: 'SSH tunnels, encrypted config export, destructive action guards' },
    { icon: Bot, label: 'AI assistant plus MCP server for Claude Code, Cursor, and agents' },
    { icon: Network, label: 'ER diagrams, schema diff, and field lineage for deeper analysis' },
    { icon: FileCode, label: 'CSV, Excel, SQL files, full exports, and cross-engine transfer' },
    { icon: Zap, label: 'Desktop app and self-hosted web deployment from the same project' },
  ],
  cn: [
    { icon: Database, label: 'Rust 原生驱动，不依赖 JDBC 运行时' },
    { icon: Shield, label: 'SSH 隧道、加密配置导出、危险操作确认' },
    { icon: Bot, label: '内置 AI 助手，以及面向 Claude Code、Cursor 的 MCP Server' },
    { icon: Network, label: 'ER 图、Schema 对比、字段血缘，覆盖更深层分析场景' },
    { icon: FileCode, label: 'CSV、Excel、SQL 文件、完整导出和跨引擎传输' },
    { icon: Zap, label: '桌面应用与自托管 Web 部署来自同一个项目' },
  ],
};

const latestUpdates = {
  en: {
    version: 'v0.5.4',
    title: 'Latest updates',
    desc: 'Mirrored from the latest GitHub release notes.',
    link: 'Read the changelog',
    items: [
      'JDBC SSH tunnels and proxy support',
      'Grouped object browser with context menus',
      'Redis batch operations and command runner',
      'LIKE / NOT LIKE filters in the data grid',
    ],
  },
  cn: {
    version: 'v0.5.4',
    title: '最近更新',
    desc: '同步 GitHub 最新 Release Notes。',
    link: '查看更新日志',
    items: [
      'JDBC SSH 隧道和代理支持',
      '对象浏览器分组与右键菜单',
      'Redis 批量操作和命令行',
      '数据表格 LIKE / NOT LIKE 过滤',
    ],
  },
};

const testimonials = {
  en: [
    {
      name: '@cyano',
      role: 'PostgreSQL and Redis workflows',
      avatar: '/avatars/cyano.jpg',
      quote: 'DBX keeps query work, schema checks, and Redis inspection in one small app. It feels focused instead of overloaded.',
    },
    {
      name: '@vbvb',
      role: 'Daily reporting',
      avatar: '/avatars/vbvb.png',
      quote: 'The data grid and export flow are the parts I reach for every day. Filters, previews, and edits stay close to the data.',
    },
    {
      name: '@ar414',
      role: 'Self-hosted tooling',
      avatar: '/avatars/ar414.jpg',
      quote: 'Desktop mode is light enough for local work, and Docker mode makes it easy to give the team browser access.',
    },
    {
      name: '@ryan',
      role: 'Multi-database projects',
      avatar: '/avatars/ryan.jpg',
      quote: 'I can jump between SQLite, MySQL, MongoDB, and DuckDB without changing tools or waiting on a heavy runtime.',
    },
    {
      name: '@acane',
      role: 'Schema review',
      avatar: '/avatars/acane.png',
      quote: 'Schema browsing, ER diagrams, and diff tools make reviews faster because the important context is already connected.',
    },
    {
      name: '@ydwang',
      role: 'Agent workflows',
      avatar: '/avatars/ydwang.png',
      quote: 'The MCP server is a practical touch. It lets coding agents inspect database context without inventing another bridge.',
    },
    {
      name: '@guangguang',
      role: 'Schema navigation',
      avatar: '/avatars/guangguang.jpg',
      quote: 'Sidebar search and grouped objects make large schemas manageable. I can find what I need without scrolling through hundreds of tables.',
    },
    {
      name: '@xuyuan',
      role: 'SQL editing',
      avatar: '/avatars/xuyuan.jpg',
      quote: 'Code completion in the SQL editor picks up column names and table aliases automatically. It saves a lot of tab-switching to check schema.',
    },
    {
      name: '@itkui',
      role: 'Data export',
      avatar: '/avatars/itkui.jpg',
      quote: "Export options cover CSV, Excel, and SQL inserts. For daily data pulls, the workflow is quick and doesn't need extra scripting.",
    },
    {
      name: '@mebiuw',
      role: 'Secure connections',
      avatar: '/avatars/mebiuw.jpg',
      quote: 'SSH tunnel setup is straightforward — fill in the fields and connect. No need to manage port forwarding manually in a terminal.',
    },
    {
      name: '@patrickz',
      role: 'Database design',
      avatar: '/avatars/patrickz.jpg',
      quote: 'ER diagrams give a clear picture of table relationships. Useful during design reviews when the team needs a shared visual reference.',
    },
    {
      name: '@yanxuecan',
      role: 'AI-assisted queries',
      avatar: '/avatars/yanxuecan.jpg',
      quote: 'The AI assistant helps draft queries from natural language. It handles routine JOINs and aggregations well enough to speed things up.',
    },
  ],
  cn: [
    {
      name: '不剪发的Tony老师',
      role: 'PostgreSQL 与 Redis 工作流',
      avatar: '/avatars/dongxuyang85.jpg',
      quote: 'DBX 把查询、结构检查和 Redis 查看放在一个轻量工具里，日常数据库工作不会被复杂界面打断。',
    },
    {
      name: 'Husky明夋',
      role: '报表与数据核对',
      avatar: '/avatars/husky.jpg',
      quote: '数据表格、过滤、预览和导出都离数据很近，用起来像是为高频操作专门整理过。',
    },
    {
      name: '孙志岗',
      role: '团队自托管工具',
      avatar: '/avatars/sunzhigang.jpg',
      quote: '本地桌面版足够轻，自托管 Web 版又方便团队共用，同一个项目覆盖了两种场景。',
    },
    {
      name: 'zhufeng',
      role: '多数据库项目',
      avatar: '/avatars/zhufeng.jpg',
      quote: 'SQLite、MySQL、MongoDB、DuckDB 来回切换不用换工具，也不用拖着很重的运行时。',
    },
    {
      name: '樱桃小财主',
      role: '结构审查',
      avatar: '/avatars/yingtao.jpg',
      quote: '结构浏览、ER 图和 Schema 对比放在一起，做 review 时上下文更完整。',
    },
    {
      name: 'momo',
      role: 'Agent 数据库上下文',
      avatar: '/avatars/momo.jpg',
      quote: 'MCP Server 很实用，能让编码 Agent 读取数据库上下文，不需要再额外搭桥。',
    },
    {
      name: '逛逛GitHub',
      role: '结构导航',
      avatar: '/avatars/guangguang.jpg',
      quote: '侧边栏搜索和分组浏览让大型 Schema 也不会迷路，不用在几百张表里翻来翻去。',
    },
    {
      name: '序员先生',
      role: 'SQL 编辑',
      avatar: '/avatars/xuyuan.jpg',
      quote: 'SQL 编辑器的补全能自动识别列名和别名，不用反复切到结构面板去确认字段。',
    },
    {
      name: 'IT老魁',
      role: '数据导出',
      avatar: '/avatars/itkui.jpg',
      quote: '导出支持 CSV、Excel 和 INSERT 语句，日常取数据很快，不用再额外写脚本。',
    },
    {
      name: 'MebiuW',
      role: '安全连接',
      avatar: '/avatars/mebiuw.jpg',
      quote: 'SSH 隧道设置很直接，填好参数就能连，不用在终端里手动转发端口。',
    },
    {
      name: 'Patrick Zhang',
      role: '数据库设计',
      avatar: '/avatars/patrickz.jpg',
      quote: 'ER 图把表关系展示得很清楚，团队做设计评审时有个共同的可视化参考。',
    },
    {
      name: '闫学灿',
      role: 'AI 辅助查询',
      avatar: '/avatars/yanxuecan.jpg',
      quote: 'AI 助手能从自然语言生成查询，常规的 JOIN 和聚合写得不错，省了不少手敲时间。',
    },
  ],
};

const i18nText = {
  en: {
    navDocs: 'Docs',
    navChangelog: 'Changelog',
    navCommunity: 'Community',
    lang: '中文',
    eyebrow: 'Open-source database workspace',
    heroTitle: 'A focused database client for daily work.',
    heroSubtitle:
      'DBX brings connections, SQL editing, data grids, schema tools, AI assistance, and self-hosted access into one lightweight product.',
    download: 'Download DBX',
    readDocs: 'Read the docs',
    docsStart: 'Start here',
    docsStartDesc: 'Install DBX, create your first connection, and learn the main workflow.',
    workflowsTitle: 'Core workflows',
    workflowsDesc: 'The docs are organized around what you actually do in a database client.',
    supportTitle: 'Supports many databases',
    supportDesc:
      'Connect and manage SQL, NoSQL, embedded databases, and MySQL/PostgreSQL-compatible engines without switching tools.',
    testimonialsTitle: 'What DBX is good at',
    testimonialsDesc: 'A closer look at the everyday database workflows DBX is built to make smoother.',
    capabilitiesTitle: 'Built for real database work',
    footerTitle: 'Ready to try DBX?',
    footerDesc: 'Use the desktop app for local work, or deploy the Docker version for browser-based access.',
    release: 'Latest release',
    docker: 'Docker setup',
  },
  cn: {
    navDocs: '文档',
    navChangelog: '更新日志',
    navCommunity: '社区',
    lang: 'English',
    eyebrow: '开源数据库工作台',
    heroTitle: '专注日常工作的数据库客户端。',
    heroSubtitle: 'DBX 将连接管理、SQL 编辑、数据表格、结构工具、AI 助手和自托管访问放进一个轻量产品里。',
    download: '下载 DBX',
    readDocs: '查看文档',
    docsStart: '从这里开始',
    docsStartDesc: '安装 DBX、创建第一个连接，并了解主要工作流。',
    workflowsTitle: '核心工作流',
    workflowsDesc: '文档围绕数据库客户端里的真实任务组织，而不是堆功能清单。',
    supportTitle: '支持多种数据库',
    supportDesc:
      '告别频繁切换工具的烦恼。DBX 可以连接和管理多种数据库类型，让你更专注于查询、分析和数据本身。',
    testimonialsTitle: 'DBX 适合什么样的工作',
    testimonialsDesc: '从连接管理、数据浏览到 AI 辅助，DBX 围绕高频数据库工作流打磨体验。',
    capabilitiesTitle: '面向真实数据库工作的能力',
    footerTitle: '准备试试 DBX？',
    footerDesc: '本地工作使用桌面版，需要浏览器访问时部署 Docker 版。',
    release: '最新版本',
    docker: 'Docker 部署',
  },
};

export default async function LandingPage({
  params,
}: {
  params: Promise<{ lang: string }>;
}) {
  const { lang } = await params;
  const l = lang === 'cn' ? 'cn' : 'en';
  const t = i18nText[l];
  const workflowItems = workflows[l];
  const capabilityItems = capabilities[l];
  const starLabel = await getGitHubStarLabel();
  const metricItems = metrics(starLabel)[l];
  const latest = latestUpdates[l];
  const testimonialItems = testimonials[l];

  return (
    <main className="landing">
      <nav className="landing-nav">
        <div className="landing-nav-inner">
          <Link href={`/${l}`} className="landing-logo">
            <img src="/logo.png" alt="DBX" width={28} height={28} />
            <span>DBX</span>
          </Link>
          <div className="landing-nav-links">
            <Link href={`/${l}/docs/what-is-dbx`} target="_blank">
              {t.navDocs}
            </Link>
            <Link href={`/${l}/docs/changelog`} target="_blank">
              {t.navChangelog}
            </Link>
            <Link href="https://github.com/t8y2/dbx" target="_blank">
              GitHub
            </Link>
            <Link href={l === 'cn' ? '/en' : '/cn'} className="landing-lang-switch">
              {t.lang}
            </Link>
          </div>
        </div>
      </nav>

      <section className="landing-hero">
        <Spotlight />
        <div className="landing-hero-copy">
          <div className="landing-eyebrow">{t.eyebrow}</div>
          <h1>{t.heroTitle}</h1>
          <p>{t.heroSubtitle}</p>
          <div className="landing-hero-cta">
            <Link href="https://github.com/t8y2/dbx/releases/latest" className="landing-btn-primary" target="_blank">
              {t.download}
              <ArrowRight size={16} />
            </Link>
            <Link href={`/${l}/docs/getting-started`} className="landing-btn-secondary" target="_blank">
              {t.readDocs}
            </Link>
          </div>
          <InstallTabs lang={l} />
        </div>
        <HeroProductStage />
      </section>

      <RevealSection className="landing-metrics">
        {metricItems.map((item) => (
          <div key={item.label}>
            <strong>{item.value}</strong>
            <span>{item.label}</span>
          </div>
        ))}
      </RevealSection>

      <RevealSection className="landing-doc-start">
        <div>
          <h2>{t.docsStart}</h2>
          <p>{t.docsStartDesc}</p>
        </div>
        <Link href={`/${l}/docs/getting-started`} className="landing-inline-link" target="_blank">
          {t.readDocs}
          <ArrowRight size={15} />
        </Link>
      </RevealSection>

      <RevealSection className="landing-section">
        <div className="landing-section-heading">
          <h2>{t.workflowsTitle}</h2>
          <p>{t.workflowsDesc}</p>
        </div>
        <div className="landing-workflow-grid">
          {workflowItems.map((item) => (
            <Link key={item.title} href={item.href} className="landing-workflow-card" target="_blank">
              <item.icon size={20} />
              <h3>{item.title}</h3>
              <p>{item.desc}</p>
              <span>
                {t.readDocs}
                <ArrowRight size={14} />
              </span>
            </Link>
          ))}
        </div>
      </RevealSection>

      <RevealSection className="landing-section landing-support">
        <div className="landing-section-heading">
          <h2>{t.supportTitle}</h2>
          <p>{t.supportDesc}</p>
        </div>
        <div className="landing-db-grid">
          {databaseSupport.map((db) => (
            <div className="landing-db-card" key={db.name} style={{ '--db-tone': db.tone } as CSSProperties}>
              <div className="landing-db-icon">
                {db.icon ? <img src={db.icon} alt="" width={42} height={42} /> : <span>{db.initials}</span>}
              </div>
              <strong>{db.name}</strong>
            </div>
          ))}
        </div>
      </RevealSection>

      <RevealSection className="landing-section landing-testimonials">
        <div className="landing-section-heading">
          <h2>{t.testimonialsTitle}</h2>
          <p>{t.testimonialsDesc}</p>
        </div>
        <div className="landing-testimonial-wall">
          <InfiniteMovingCards items={testimonialItems.slice(0, 6)} speed="slow" />
          <InfiniteMovingCards items={testimonialItems.slice(6)} direction="right" speed="slow" />
        </div>
      </RevealSection>

      <RevealSection className="landing-section">
        <div className="landing-section-heading">
          <h2>{t.capabilitiesTitle}</h2>
        </div>
        <div className="landing-capability-grid">
          {capabilityItems.map((item) => (
            <div key={item.label} className="landing-capability">
              <item.icon size={18} />
              <span>{item.label}</span>
            </div>
          ))}
        </div>
      </RevealSection>

      <RevealSection className="landing-updates">
        <div className="landing-update-version">{latest.version}</div>
        <div className="landing-update-copy">
          <h2>{latest.title}</h2>
          <p>{latest.desc}</p>
        </div>
        <ul className="landing-update-list">
          {latest.items.map((item) => (
            <li key={item}>
              <CheckCircle2 size={14} />
              <span>{item}</span>
            </li>
          ))}
        </ul>
        <Link href={`/${l}/docs/changelog`} className="landing-inline-link" target="_blank">
          {latest.link}
          <ArrowRight size={15} />
        </Link>
      </RevealSection>

      <RevealSection className="landing-final">
        <div>
          <h2>{t.footerTitle}</h2>
          <p>{t.footerDesc}</p>
        </div>
        <div className="landing-final-actions">
          <Link href="https://github.com/t8y2/dbx/releases/latest" target="_blank">
            {t.release}
          </Link>
          <Link href={`/${l}/docs/getting-started#docker`} target="_blank">
            {t.docker}
          </Link>
        </div>
      </RevealSection>
    </main>
  );
}
