import type { AddAccountRequest } from "@/lib/api";

/** Server fields a provider preset fills in. */
export type ServerSettings = Pick<
  AddAccountRequest,
  "imap_host" | "imap_port" | "smtp_host" | "smtp_port" | "imap_security" | "smtp_security"
>;

export interface MailProvider {
  id: string;
  /** Display name (brand names stay untranslated). */
  name: string;
  /** Exact domains this provider serves (lower case). */
  domains: string[];
  settings: ServerSettings;
  /** Sign-in the provider expects instead of the mailbox password. */
  auth?: "google-oauth" | "microsoft-oauth" | "app-password";
}

const ssl = (imap: string, smtp: string, smtpPort = 465): ServerSettings => ({
  imap_host: imap,
  imap_port: 993,
  imap_security: "tls",
  smtp_host: smtp,
  smtp_port: smtpPort,
  smtp_security: smtpPort === 587 ? "starttls" : "tls",
});

/*
 * Sources: Mozilla Thunderbird ISPDB (autoconfig.thunderbird.net), each
 * provider's own help pages, and for providers whose help pages show no
 * server values (139, Sohu, 88, NetEase VIP/188) a live TLS + greeting check. Only exact domains are matched; company and
 * school domains hosted by these providers are left to manual setup.
 */
const OUTLOOK_DOMAINS = (
  "hotmail.com live.com msn.com outlook.com windowslive.com outlook.at outlook.be outlook.cl "
  + "outlook.cz outlook.de outlook.dk outlook.es outlook.fr outlook.hu outlook.ie outlook.in "
  + "outlook.it outlook.jp outlook.kr outlook.lv outlook.my outlook.ph outlook.pt outlook.sa "
  + "outlook.sg outlook.sk outlook.co.id outlook.co.il outlook.co.th outlook.com.ar outlook.com.au "
  + "outlook.com.br outlook.com.gr outlook.com.tr outlook.com.vn hotmail.be hotmail.ca hotmail.cl "
  + "hotmail.cz hotmail.de hotmail.dk hotmail.es hotmail.fi hotmail.fr hotmail.gr hotmail.hu "
  + "hotmail.it hotmail.lt hotmail.lv hotmail.my hotmail.nl hotmail.no hotmail.ph hotmail.rs "
  + "hotmail.se hotmail.sg hotmail.sk hotmail.co.id hotmail.co.il hotmail.co.in hotmail.co.jp "
  + "hotmail.co.kr hotmail.co.th hotmail.co.uk hotmail.co.za hotmail.com.ar hotmail.com.au "
  + "hotmail.com.br hotmail.com.hk hotmail.com.tr hotmail.com.tw hotmail.com.vn live.at live.be "
  + "live.ca live.cl live.cn live.de live.dk live.fi live.fr live.hk live.ie live.in live.it "
  + "live.jp live.nl live.no live.ru live.se live.co.jp live.co.kr live.co.uk live.co.za "
  + "live.com.ar live.com.au live.com.mx live.com.my live.com.ph live.com.pt live.com.sg livemail.tw"
).split(" ");

export const MAIL_PROVIDERS: MailProvider[] = [
  {
    id: "gmail",
    name: "Gmail",
    domains: ["gmail.com", "googlemail.com"],
    settings: ssl("imap.gmail.com", "smtp.gmail.com", 587),
    auth: "google-oauth",
  },
  {
    id: "outlook",
    name: "Outlook",
    domains: OUTLOOK_DOMAINS,
    settings: ssl("outlook.office365.com", "smtp.office365.com", 587),
    auth: "microsoft-oauth",
  },
  {
    id: "icloud",
    name: "iCloud",
    domains: ["icloud.com", "me.com", "mac.com"],
    settings: ssl("imap.mail.me.com", "smtp.mail.me.com", 587),
    auth: "app-password",
  },
  {
    id: "qq",
    name: "QQ",
    // foxmail.com and vip.qq.com are QQ Mail addresses on the same servers.
    domains: ["qq.com", "foxmail.com", "vip.qq.com"],
    settings: ssl("imap.qq.com", "smtp.qq.com"),
    auth: "app-password",
  },
  {
    id: "163",
    name: "163",
    domains: ["163.com"],
    settings: ssl("imap.163.com", "smtp.163.com"),
    auth: "app-password",
  },
  {
    id: "126",
    name: "126",
    domains: ["126.com"],
    settings: ssl("imap.126.com", "smtp.126.com"),
    auth: "app-password",
  },
  {
    id: "yeah",
    name: "Yeah",
    domains: ["yeah.net"],
    settings: ssl("imap.yeah.net", "smtp.yeah.net"),
    auth: "app-password",
  },
  {
    id: "vip163",
    name: "163 VIP",
    domains: ["vip.163.com"],
    settings: ssl("imap.vip.163.com", "smtp.vip.163.com"),
    auth: "app-password",
  },
  {
    id: "188",
    name: "188",
    domains: ["188.com"],
    settings: ssl("imap.188.com", "smtp.188.com"),
    auth: "app-password",
  },
  {
    id: "139",
    name: "139",
    domains: ["139.com"],
    settings: ssl("imap.139.com", "smtp.139.com"),
    auth: "app-password",
  },
  {
    id: "189",
    name: "189",
    domains: ["189.cn"],
    settings: ssl("imap.189.cn", "smtp.189.cn"),
    auth: "app-password",
  },
  {
    // From the #36 report; hostnames exist, ports not independently verified.
    id: "wo",
    name: "Wo",
    domains: ["wo.cn"],
    settings: ssl("imap.wo.cn", "smtp.wo.cn"),
    auth: "app-password",
  },
  {
    id: "21cn",
    name: "21CN",
    domains: ["21cn.com"],
    settings: ssl("imap.21cn.com", "smtp.21cn.com"),
    auth: "app-password",
  },
  {
    id: "aliyun",
    name: "Aliyun",
    domains: ["aliyun.com"],
    settings: ssl("imap.aliyun.com", "smtp.aliyun.com"),
  },
  {
    id: "sina-com",
    name: "Sina",
    domains: ["sina.com"],
    settings: ssl("imap.sina.com", "smtp.sina.com"),
    auth: "app-password",
  },
  {
    id: "sina-cn",
    name: "Sina",
    domains: ["sina.cn"],
    settings: ssl("imap.sina.cn", "smtp.sina.cn"),
    auth: "app-password",
  },
  {
    id: "sina-vip-com",
    name: "Sina VIP",
    domains: ["vip.sina.com"],
    settings: ssl("imap.vip.sina.com", "smtp.vip.sina.com"),
    auth: "app-password",
  },
  {
    id: "sina-vip-cn",
    name: "Sina VIP",
    domains: ["vip.sina.cn"],
    settings: ssl("imap.vip.sina.cn", "smtp.vip.sina.cn"),
    auth: "app-password",
  },
  {
    id: "sohu",
    name: "Sohu",
    domains: ["sohu.com"],
    settings: ssl("imap.sohu.com", "smtp.sohu.com"),
    auth: "app-password",
  },
  {
    id: "88",
    name: "88",
    domains: ["88.com"],
    settings: ssl("imap.88.com", "smtp.88.com"),
    auth: "app-password",
  },
  {
    id: "fastmail",
    name: "Fastmail",
    domains: ["fastmail.com"],
    settings: ssl("imap.fastmail.com", "smtp.fastmail.com"),
    auth: "app-password",
  },
  {
    id: "mailru",
    name: "Mail.ru",
    domains: ["mail.ru"],
    settings: ssl("imap.mail.ru", "smtp.mail.ru"),
    auth: "app-password",
  },
  {
    id: "yahoo",
    name: "Yahoo",
    domains: [
      "yahoo.com", "yahoo.ca", "yahoo.de", "yahoo.it", "yahoo.fr", "yahoo.es", "yahoo.se",
      "yahoo.co.in", "yahoo.co.uk", "yahoo.co.nz", "yahoo.com.au", "yahoo.com.ar", "yahoo.com.br",
      "yahoo.com.mx", "ymail.com", "myyahoo.com", "rocketmail.com",
    ],
    settings: ssl("imap.mail.yahoo.com", "smtp.mail.yahoo.com"),
    auth: "app-password",
  },
  {
    id: "aol",
    name: "AOL",
    domains: ["aol.com", "aim.com", "aol.de", "aol.it", "aol.fr", "aol.es", "aol.se", "aol.co.uk"],
    settings: ssl("imap.aol.com", "smtp.aol.com"),
    auth: "app-password",
  },
  {
    id: "gmx-com",
    name: "GMX",
    domains: ["gmx.com", "gmx.us", "gmx.co.uk", "gmx.es", "gmx.fr", "gmx.ca", "gmx.it"],
    settings: ssl("imap.gmx.com", "mail.gmx.com"),
  },
  {
    id: "gmx-net",
    name: "GMX",
    domains: ["gmx.net", "gmx.de", "gmx.at", "gmx.ch"],
    settings: ssl("imap.gmx.net", "mail.gmx.net"),
  },
  {
    id: "zoho",
    name: "Zoho",
    domains: ["zoho.com", "zohomail.com"],
    settings: ssl("imap.zoho.com", "smtp.zoho.com"),
  },
  {
    id: "yandex",
    name: "Yandex",
    domains: ["yandex.ru", "yandex.com", "yandex.by", "yandex.kz", "yandex.ua", "ya.ru"],
    settings: ssl("imap.yandex.com", "smtp.yandex.com"),
    auth: "app-password",
  },
];

/** The four providers shown as quick-setup buttons. */
export const QUICK_SETUP_PROVIDER_IDS = ["gmail", "outlook", "qq", "163"] as const;

const BY_DOMAIN = new Map<string, MailProvider>();
for (const provider of MAIL_PROVIDERS) {
  for (const domain of provider.domains) BY_DOMAIN.set(domain, provider);
}

export function providerById(id: string): MailProvider | undefined {
  return MAIL_PROVIDERS.find((provider) => provider.id === id);
}

/** The provider serving this address's domain, matched exactly. */
export function providerForEmail(email: string): MailProvider | null {
  const at = email.lastIndexOf("@");
  if (at < 1) return null;
  const domain = email.slice(at + 1).trim().toLowerCase().replace(/\.$/, "");
  if (!domain) return null;
  return BY_DOMAIN.get(domain) ?? null;
}
