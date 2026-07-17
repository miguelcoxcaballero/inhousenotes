import type { GoogleAuth } from './driveClient';

const CALENDAR_BASE_URL = 'https://www.googleapis.com/calendar/v3';

export interface CalendarEventRecord {
  id: string;
  dateKey: string;
  title: string;
  time: string;
}

interface GoogleCalendarEvent {
  id: string;
  summary?: string;
  start?: { date?: string; dateTime?: string };
}

export class CalendarClient {
  constructor(private auth: GoogleAuth) {}

  isSignedIn(): boolean {
    return this.auth.isSignedIn();
  }

  async listMonth(month: Date): Promise<CalendarEventRecord[]> {
    const first = new Date(month.getFullYear(), month.getMonth(), 1);
    const next = new Date(month.getFullYear(), month.getMonth() + 1, 1);
    const params = new URLSearchParams({
      timeMin: first.toISOString(),
      timeMax: next.toISOString(),
      singleEvents: 'true',
      orderBy: 'startTime',
      maxResults: '2500'
    });
    const data = await this.request<{ items?: GoogleCalendarEvent[] }>(`/calendars/primary/events?${params}`);
    return (data.items ?? []).map((item) => {
      const raw = item.start?.dateTime ?? item.start?.date ?? '';
      const date = raw ? new Date(raw) : first;
      return {
        id: item.id,
        dateKey: item.start?.date ?? dateKey(date),
        title: item.summary ?? '(No title)',
        time: item.start?.dateTime ? timeKey(date) : ''
      };
    });
  }

  async createEvent(date: string, title: string, time: string): Promise<CalendarEventRecord> {
    const body = time
      ? {
          summary: title,
          start: { dateTime: new Date(`${date}T${time}:00`).toISOString() },
          end: { dateTime: new Date(new Date(`${date}T${time}:00`).getTime() + 60 * 60 * 1000).toISOString() }
        }
      : {
          summary: title,
          start: { date },
          end: { date: addDays(date, 1) }
        };
    const saved = await this.request<GoogleCalendarEvent>('/calendars/primary/events', {
      method: 'POST',
      body
    });
    return { id: saved.id, dateKey: date, title, time };
  }

  async deleteEvent(eventId: string): Promise<void> {
    await this.request<void>(`/calendars/primary/events/${encodeURIComponent(eventId)}`, { method: 'DELETE' });
  }

  private async request<T>(path: string, init: { method?: string; body?: unknown } = {}): Promise<T> {
    const accessToken = await this.auth.getAccessToken();
    const resp = await fetch(`${CALENDAR_BASE_URL}${path}`, {
      method: init.method ?? 'GET',
      headers: {
        Authorization: `Bearer ${accessToken}`,
        ...(init.body ? { 'Content-Type': 'application/json' } : {})
      },
      body: init.body ? JSON.stringify(init.body) : undefined
    });
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({}));
      throw new Error((err as { error?: { message?: string } }).error?.message ?? `Calendar API ${resp.status}`);
    }
    if (resp.status === 204) return undefined as T;
    return resp.json() as Promise<T>;
  }
}

function dateKey(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

function timeKey(date: Date): string {
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}

function addDays(date: string, days: number): string {
  const [y, m, d] = date.split('-').map(Number);
  return dateKey(new Date(y!, m! - 1, d! + days));
}
