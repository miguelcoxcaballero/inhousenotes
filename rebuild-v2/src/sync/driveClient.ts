// Google Drive API client using PKCE OAuth2 with refresh token support.
// Handles token management, auto-refresh, and all Drive API operations.

import { newId } from '../core/ids';

// â”€â”€ Token types â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

export interface TokenResponse {
  access_token: string;
  refresh_token?: string;
  expires_in: number;
  token_type: string;
}

export interface StoredTokens {
  accessToken: string;
  refreshToken: string;
  expiresAt: number; // Unix ms timestamp
}

const TOKEN_KEY = 'ihn_drive_tokens';
const CLIENT_ID_KEY = 'ihn_drive_client_id';
// Embedded OAuth client (same project as the legacy app).
const DEFAULT_CLIENT_ID = '435784295430-cmug30o42f1vu4ijgor9sjb0ro4oo37o.apps.googleusercontent.com';
const CODE_VERIFIER_KEY = 'ihn_pkce_verifier';

// â”€â”€ PKCE helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

async function sha256(plain: string): Promise<ArrayBuffer> {
  const encoder = new TextEncoder();
  const data = encoder.encode(plain);
  return crypto.subtle.digest('SHA-256', data);
}

function base64UrlEncode(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]!);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

async function generateCodeVerifier(): Promise<string> {
  const bytes = new Uint8Array(64);
  crypto.getRandomValues(bytes);
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~';
  let out = '';
  for (let i = 0; i < 64; i++) {
    out += alphabet[bytes[i]! % alphabet.length]!;
  }
  return out;
}

// â”€â”€ GoogleAuth â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

export class GoogleAuth {
  private tokens: StoredTokens | null = null;
  private clientId: string | null = null;
  private refreshTimeout: ReturnType<typeof setTimeout> | null = null;

  onTokensChange: ((tokens: StoredTokens | null) => void) | null = null;
  /** Fired when an auth error needs user intervention (e.g., refresh token expired). */
  onAuthError: ((error: Error) => void) | null = null;

  constructor() {
    this.loadStoredTokens();
  }

  private loadStoredTokens(): void {
    try {
      const raw = localStorage.getItem(TOKEN_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as Partial<StoredTokens>;
        if (isValidStoredTokens(parsed)) {
          this.tokens = parsed;
        } else {
          localStorage.removeItem(TOKEN_KEY);
          this.tokens = null;
        }
      }
      this.clientId = localStorage.getItem(CLIENT_ID_KEY) ?? DEFAULT_CLIENT_ID;
    } catch {
      this.tokens = null;
      localStorage.removeItem(TOKEN_KEY);
    }
  }

  private saveTokens(tokens: StoredTokens): void {
    this.tokens = tokens;
    localStorage.setItem(TOKEN_KEY, JSON.stringify(tokens));
    this.onTokensChange?.(tokens);
  }

  isSignedIn(): boolean {
    return this.tokens !== null && this.tokens.expiresAt > Date.now();
  }

  hasStoredTokens(): boolean {
    return this.tokens !== null;
  }

  getClientId(): string | null {
    return this.clientId;
  }

  setClientId(clientId: string): void {
    this.clientId = clientId;
    localStorage.setItem(CLIENT_ID_KEY, clientId);
  }

  /**
   * Initiate PKCE OAuth2 sign-in. Opens a popup window for the user to
   * authorize the app. Returns a promise that resolves when the auth flow
   * completes successfully.
   */
  async signIn(): Promise<void> {
    if (!this.clientId) throw new Error('Client ID not configured');

    const codeVerifier = await generateCodeVerifier();
    const codeChallenge = base64UrlEncode(await sha256(codeVerifier));

    sessionStorage.setItem(CODE_VERIFIER_KEY, codeVerifier);

    const redirectUri = `${window.location.origin}/oauth-callback`;
    const params = new URLSearchParams({
      client_id: this.clientId,
      redirect_uri: redirectUri,
      response_type: 'code',
      scope: [
        'https://www.googleapis.com/auth/drive.file',
        'https://www.googleapis.com/auth/calendar.events'
      ].join(' '),
      code_challenge_method: 'S256',
      code_challenge: codeChallenge,
      access_type: 'offline',
      prompt: 'consent'
    });

    const authUrl = `https://accounts.google.com/o/oauth2/v2/auth?${params}`;

    // Open popup window
    const popup = window.open(authUrl, 'oauth', 'width=600,height=700,left=100,top=100');
    if (!popup) throw new Error('Popup blocked');

    return new Promise((resolve, reject) => {
      // Listen for the callback via postMessage
      const messageHandler = async (event: MessageEvent) => {
        if (event.origin !== window.location.origin) return;
        if (!event.data?.code) return;

        window.removeEventListener('message', messageHandler);
        popup.close();

        try {
          const code = event.data.code as string;
          const verifier = sessionStorage.getItem(CODE_VERIFIER_KEY) ?? '';
          sessionStorage.removeItem(CODE_VERIFIER_KEY);

          const tokens = await this.exchangeCode(code, verifier, redirectUri);
          this.saveTokens(tokens);
          resolve();
        } catch (err) {
          reject(err);
        }
      };

      window.addEventListener('message', messageHandler);

      // Fallback: check if popup closed without completing
      const pollClosed = setInterval(() => {
        if (popup.closed) {
          clearInterval(pollClosed);
          window.removeEventListener('message', messageHandler);
          reject(new Error('Sign-in cancelled'));
        }
      }, 500);
    });
  }

  private async exchangeCode(code: string, codeVerifier: string, redirectUri: string): Promise<StoredTokens> {
    const resp = await fetch('https://oauth2.googleapis.com/token', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        client_id: this.clientId ?? '',
        code,
        code_verifier: codeVerifier,
        grant_type: 'authorization_code',
        redirect_uri: redirectUri
      })
    });

    if (!resp.ok) {
      const err = await resp.json().catch(() => ({}));
      throw new Error(`Token exchange failed: ${(err as { error?: string }).error ?? resp.status}`);
    }

    const data = (await resp.json()) as TokenResponse;
    if (!data.refresh_token) throw new Error('No refresh token received');

    return {
      accessToken: data.access_token,
      refreshToken: data.refresh_token,
      expiresAt: Date.now() + data.expires_in * 1000
    };
  }

  /**
   * Get a valid access token, refreshing if necessary.
   * Throws if refresh fails (e.g., revoked permissions).
   */
  async getAccessToken(): Promise<string> {
    if (!this.tokens) throw new Error('Not signed in');

    // Refresh if expiring within 5 minutes
    if (this.tokens.expiresAt - Date.now() < 5 * 60 * 1000) {
      await this.refresh();
    }

    return this.tokens.accessToken;
  }

  private async refresh(): Promise<void> {
    if (!this.tokens?.refreshToken || !this.clientId) {
      throw new Error('Cannot refresh: no refresh token or client ID');
    }

    const resp = await fetch('https://oauth2.googleapis.com/token', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        client_id: this.clientId,
        refresh_token: this.tokens.refreshToken,
        grant_type: 'refresh_token'
      })
    });

    if (!resp.ok) {
      const err = await resp.json().catch(() => ({}));
      const error = new Error(`Token refresh failed: ${(err as { error?: string }).error ?? resp.status}`);
      this.onAuthError?.(error);
      throw error;
    }

    const data = (await resp.json()) as TokenResponse;
    this.saveTokens({
      accessToken: data.access_token,
      refreshToken: this.tokens.refreshToken,
      expiresAt: Date.now() + data.expires_in * 1000
    });
  }

  signOut(): void {
    if (this.refreshTimeout) clearTimeout(this.refreshTimeout);
    this.tokens = null;
    localStorage.removeItem(TOKEN_KEY);
    this.onTokensChange?.(null);
  }
}

function isValidStoredTokens(tokens: Partial<StoredTokens>): tokens is StoredTokens {
  if (typeof tokens.accessToken !== 'string' || tokens.accessToken.length < 20) return false;
  if (typeof tokens.refreshToken !== 'string' || tokens.refreshToken.length < 20) return false;
  if (!Number.isFinite(tokens.expiresAt) || Number(tokens.expiresAt) <= Date.now()) return false;
  if (/dummy|demo|fake|developer/i.test(`${tokens.accessToken} ${tokens.refreshToken}`)) return false;
  return true;
}

// â”€â”€ DriveClient â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

const BASE_URL = 'https://www.googleapis.com/drive/v3';
const UPLOAD_URL = 'https://www.googleapis.com/upload/drive/v3';
const DRIVE_FILE_FIELDS = 'id,name,mimeType,modifiedTime,size,parents,shared,webContentLink,webViewLink,resourceKey';
const SIMPLE_UPLOAD_MAX_BYTES = 5 * 1024 * 1024;

export interface DriveFile {
  id: string;
  name: string;
  mimeType: string;
  modifiedTime: string;
  size?: string;
  parents?: string[];
  shared?: boolean;
  webContentLink?: string;
  webViewLink?: string;
  resourceKey?: string;
}

export interface DriveError {
  code: number;
  message: string;
  errors?: { domain: string; reason: string; message: string }[];
}

async function driveResponseError(response: Response, fallback: string): Promise<Error> {
  const detail = (await response.json().catch(() => ({}))) as Partial<DriveError>;
  return Object.assign(new Error(detail.message ?? fallback), {
    code: response.status,
    err: detail
  });
}

export class DriveClient {
  private auth: GoogleAuth;
  private onError: (err: Error) => void;
  private abortControllers = new Map<string, AbortController>();

  constructor(auth: GoogleAuth, onError: (err: Error) => void) {
    this.auth = auth;
    this.onError = onError;
  }

  isSignedIn(): boolean {
    return this.auth.isSignedIn();
  }

  private async request<T>(
    method: string,
    url: string,
    options: {
      body?: unknown;
      query?: Record<string, string>;
      signal?: AbortSignal;
      signalId?: string;
      headers?: Record<string, string>;
      uploadType?: 'media' | 'multipart' | 'resumable';
    } = {}
  ): Promise<T> {
    const { body, query, signal, signalId, headers: extraHeaders, uploadType } = options;

    if (signalId) {
      const ctrl = new AbortController();
      this.abortControllers.set(signalId, ctrl);
      if (signal) {
        signal.addEventListener('abort', () => ctrl.abort());
      }
    }

    try {
      const accessToken = await this.auth.getAccessToken();
      const params = new URLSearchParams(query ?? {});
      const queryString = params.toString() ? `?${params}` : '';

      const requestHeaders: Record<string, string> = {
        Authorization: `Bearer ${accessToken}`,
        ...extraHeaders
      };

      if (uploadType) {
        requestHeaders['X-Upload-Content-Type'] = uploadType === 'media' ? 'application/json' : 'multipart/related';
      }

      const resp = await fetch(`${url}${queryString}`, {
        method,
        headers: requestHeaders,
        body: body ? (typeof body === 'string' || body instanceof Blob ? body as BodyInit : JSON.stringify(body)) : undefined,
        signal: signalId ? this.abortControllers.get(signalId)?.signal : signal
      });

      if (!resp.ok) {
        const err = (await resp.json().catch(() => ({}))) as DriveError;
        throw Object.assign(new Error(err.message ?? 'Drive API error'), { code: resp.status, err });
      }

      if (resp.status === 204) return undefined as unknown as T;
      return resp.json() as Promise<T>;
    } finally {
      if (signalId) this.abortControllers.delete(signalId);
    }
  }

  /** Perform a GET request with optional query parameters. */
  async get<T>(path: string, query?: Record<string, string>): Promise<T> {
    return this.request<T>('GET', `${BASE_URL}${path}`, { query });
  }

  /** Perform a POST request with a JSON body. */
  async post<T>(path: string, body: unknown, query?: Record<string, string>): Promise<T> {
    return this.request<T>('POST', `${BASE_URL}${path}`, { body, query });
  }

  /** Perform a PUT request with a JSON body. */
  async put<T>(path: string, body: unknown, query?: Record<string, string>): Promise<T> {
    return this.request<T>('PUT', `${BASE_URL}${path}`, { body, query });
  }

  /** Perform a PATCH request with a JSON body. */
  async patch<T>(path: string, body: unknown, query?: Record<string, string>): Promise<T> {
    return this.request<T>('PATCH', `${BASE_URL}${path}`, { body, query });
  }

  /** Perform a DELETE request. */
  async delete(path: string): Promise<void> {
    return this.request<void>('DELETE', `${BASE_URL}${path}`);
  }

  /**
   * Upload media content (simple upload, max 5 MB).
   * Use uploadMedia for larger files.
   */
  async uploadMedia(
    path: string,
    blob: Blob,
    mimeType: string,
    options?: { signalId?: string; signal?: AbortSignal }
  ): Promise<DriveFile> {
    const { signalId, signal } = options ?? {};
    const accessToken = await this.auth.getAccessToken();

    const resp = await fetch(`${UPLOAD_URL}${path}?uploadType=media`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${accessToken}`,
        'Content-Type': mimeType
      },
      body: blob,
      signal: signalId ? this.abortControllers.get(signalId)?.signal : signal
    });

    if (!resp.ok) {
      const err = (await resp.json().catch(() => ({}))) as DriveError;
      throw Object.assign(new Error(err.message ?? 'Upload failed'), { code: resp.status, err });
    }

    return resp.json() as Promise<DriveFile>;
  }

  async createFile(name: string, blob: Blob, mimeType: string, parents: string[] = []): Promise<DriveFile> {
    const metadata = {
      name,
      mimeType,
      ...(parents.length > 0 ? { parents } : {})
    };
    if (blob.size > SIMPLE_UPLOAD_MAX_BYTES) {
      return this.resumableUpload('POST', `${UPLOAD_URL}/files`, metadata, blob, mimeType);
    }
    const accessToken = await this.auth.getAccessToken();
    const boundary = `ihn_${newId()}`;
    const body = new Blob([
      `--${boundary}\r\n`,
      'Content-Type: application/json; charset=UTF-8\r\n\r\n',
      JSON.stringify(metadata),
      `\r\n--${boundary}\r\n`,
      `Content-Type: ${mimeType}\r\n\r\n`,
      blob,
      `\r\n--${boundary}--`
    ], { type: `multipart/related; boundary=${boundary}` });

    const resp = await fetch(`${UPLOAD_URL}/files?uploadType=multipart&fields=id,name,mimeType,modifiedTime,size,parents,shared,webContentLink,webViewLink,resourceKey`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${accessToken}` },
      body
    });

    if (!resp.ok) {
      const err = (await resp.json().catch(() => ({}))) as DriveError;
      throw Object.assign(new Error(err.message ?? 'Create file failed'), { code: resp.status, err });
    }
    return resp.json() as Promise<DriveFile>;
  }

  async updateFileMedia(fileId: string, blob: Blob, mimeType: string): Promise<DriveFile> {
    if (blob.size > SIMPLE_UPLOAD_MAX_BYTES) {
      return this.resumableUpload('PATCH', `${UPLOAD_URL}/files/${fileId}`, {}, blob, mimeType);
    }
    const accessToken = await this.auth.getAccessToken();
    const resp = await fetch(`${UPLOAD_URL}/files/${fileId}?uploadType=media&fields=id,name,mimeType,modifiedTime,size,parents,shared,webContentLink,webViewLink,resourceKey`, {
      method: 'PATCH',
      headers: {
        Authorization: `Bearer ${accessToken}`,
        'Content-Type': mimeType
      },
      body: blob
    });

    if (!resp.ok) {
      const err = (await resp.json().catch(() => ({}))) as DriveError;
      throw Object.assign(new Error(err.message ?? 'Update file failed'), { code: resp.status, err });
    }
    return resp.json() as Promise<DriveFile>;
  }

  private async resumableUpload(
    method: 'POST' | 'PATCH',
    url: string,
    metadata: Record<string, unknown>,
    blob: Blob,
    mimeType: string
  ): Promise<DriveFile> {
    const accessToken = await this.auth.getAccessToken();
    const session = await fetch(`${url}?uploadType=resumable&fields=${DRIVE_FILE_FIELDS}`, {
      method,
      headers: {
        Authorization: `Bearer ${accessToken}`,
        'Content-Type': 'application/json; charset=UTF-8',
        'X-Upload-Content-Type': mimeType,
        'X-Upload-Content-Length': String(blob.size)
      },
      body: JSON.stringify(metadata)
    });
    if (!session.ok) throw await driveResponseError(session, 'Could not start resumable upload');
    const location = session.headers.get('Location');
    if (!location) throw new Error('Drive did not return a resumable upload URL');

    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const response = await fetch(location, {
          method: 'PUT',
          headers: { 'Content-Type': mimeType },
          body: blob
        });
        if (response.ok) return response.json() as Promise<DriveFile>;
        if (response.status < 500 || attempt === 2) {
          throw await driveResponseError(response, 'Resumable upload failed');
        }
      } catch (err) {
        if (attempt === 2) throw err;
      }
      await new Promise((resolve) => setTimeout(resolve, 300 * 2 ** attempt));
    }
    throw new Error('Resumable upload failed');
  }

  /**
   * Download file content as a Blob.
   */
  async downloadMedia(fileId: string, options?: { signalId?: string; signal?: AbortSignal; resourceKey?: string | null }): Promise<Blob> {
    const { signalId, signal, resourceKey } = options ?? {};
    const accessToken = await this.auth.getAccessToken();

    const resp = await fetch(`${BASE_URL}/files/${fileId}?alt=media`, {
      headers: {
        Authorization: `Bearer ${accessToken}`,
        ...(resourceKey ? { 'X-Goog-Drive-Resource-Keys': `${fileId}/${resourceKey}` } : {})
      },
      signal: signalId ? this.abortControllers.get(signalId)?.signal : signal
    });

    if (!resp.ok) {
      const err = (await resp.json().catch(() => ({}))) as DriveError;
      throw Object.assign(new Error(err.message ?? 'Download failed'), { code: resp.status, err });
    }

    const contentType = resp.headers.get('Content-Type') ?? 'application/octet-stream';
    return resp.blob().then((buf) => new Blob([buf], { type: contentType }));
  }

  /**
   * List files, optionally within a specific folder.
   */
  async listFiles(query?: {
    folderId?: string;
    mimeType?: string;
    pageSize?: number;
    pageToken?: string;
  }): Promise<{ files: DriveFile[]; nextPageToken?: string }> {
    const q: string[] = [];
    if (query?.folderId) q.push(`'${query.folderId}' in parents`);
    if (query?.mimeType) q.push(`mimeType = '${query.mimeType}'`);
    q.push('trashed = false');

    return this.get<{ files: DriveFile[]; nextPageToken?: string }>('/files', {
      q: q.join(' and '),
      fields: 'files(id,name,mimeType,modifiedTime,size,parents,shared,webContentLink,webViewLink,resourceKey),nextPageToken',
      pageSize: String(query?.pageSize ?? 100),
      pageToken: query?.pageToken ?? ''
    });
  }

  /**
   * Get or create the app-specific folder in Drive.
   * Returns the folder file object.
   */
  async getOrCreateAppFolder(): Promise<DriveFile> {
    const APP_FOLDER_NAME = 'Inhouse Notes';

    // Try to find existing folder
    const existing = await this.get<{ files: DriveFile[] }>('/files', {
      q: `name = '${APP_FOLDER_NAME}' and mimeType = 'application/vnd.google-apps.folder' and trashed = false`,
      spaces: 'drive',
      fields: 'files(id,name)'
    });

    if (existing.files.length > 0) {
      return existing.files[0]!;
    }

    // Create the folder
    return this.post<DriveFile>('/files', {
      name: APP_FOLDER_NAME,
      mimeType: 'application/vnd.google-apps.folder',
      parents: ['root']
    });
  }

  /** Cancel an in-progress upload/download by signal ID. */
  cancel(signalId: string): void {
    this.abortControllers.get(signalId)?.abort();
    this.abortControllers.delete(signalId);
  }
}
