export type SyncSource = "favorites" | "likes";
export type ContentTypeFilter = "all" | "article" | "video";
export type DownloadStatusFilter = "downloaded" | "pending" | "failed";

export interface AppBootstrap {
  dbPath: string;
  dataDir: string;
  downloadDir: string;
  downloadThreads: number;
  activeSessionIds: string[];
  version: string;
  buildDate: string;
  buildTime: string;
  buildLabel: string;
  appTitle: string;
}

export interface SyncSession {
  id: string;
  source: SyncSource;
  status: string;
  mode: string;
  totalCandidates: number;
  totalSkipped: number;
  totalDiscovered: number;
  totalSaved: number;
  totalDownloaded: number;
  message: string;
  startedAt: string;
  finishedAt?: string | null;
}

export interface SyncEvent {
  id: number;
  sessionId: string;
  level: "info" | "error";
  message: string;
  createdAt: string;
}

export interface SyncStartRequest {
  source: SyncSource;
  mode: "list" | "download" | "incremental" | "verify";
  maxItems?: number;
}

export interface DiagnosePageRequest {
  source: SyncSource;
}

export interface PageDiagnosis {
  ok: boolean;
  source: SyncSource;
  message: string;
  pageUrl?: string | null;
  pageTitle?: string | null;
  logs: string[];
}

export interface LoginStatus {
  loggedIn: boolean;
  loginRequired: boolean;
  source: SyncSource;
  message: string;
  pageUrl?: string | null;
  pageTitle?: string | null;
}

export interface ContentItem {
  id: number;
  remoteId: string;
  source: SyncSource;
  title: string;
  summary: string;
  contentText: string;
  author: string;
  contentType: string;
  sourceUrl: string;
  coverUrl?: string | null;
  coverPath?: string | null;
  articlePath?: string | null;
  videoPath?: string | null;
  localDir?: string | null;
  listOrder?: number | null;
  syncedAt: string;
  downloaded: boolean;
  downloadError?: string | null;
  rawJson: string;
  tags: string[];
}

export interface PagedContentItems {
  items: ContentItem[];
  total: number;
  page: number;
  pageSize: number;
}

export interface TagOption {
  name: string;
  count: number;
}

export interface UserProfile {
  name: string;
  avatarUrl?: string | null;
  likes: string;
  followers: string;
  following: string;
  bio: string;
  updatedAt: string;
}
