import { createPage } from '../core/model';
import type { DocStore } from '../core/store';
import type { DocRenderer } from '../render/docRenderer';
import { serializePage } from '../core/serial';
import type { CalendarClient } from '../sync/calendarClient';
import { makeOverlay, buttonStyle, smallText } from './modal';

interface CalendarEvent {
  id: string;
  dateKey: string;
  title: string;
  time: string;
  source?: 'local' | 'google';
}

export class CalendarPanel {
  private cursor = startOfMonth(new Date());

  constructor(
    private host: HTMLElement,
    private store: DocStore,
    private renderer: DocRenderer,
    private calendar: CalendarClient | null = null
  ) {}

  open(): void {
    const modal = makeOverlay('Calendar');
    this.host.appendChild(modal.overlay);
    this.render(modal.body);
  }

  private render(body: HTMLElement): void {
    body.innerHTML = '';
    const events = this.loadEvents();
    const header = document.createElement('div');
    Object.assign(header.style, {
      display: 'flex',
      justifyContent: 'space-between',
      gap: '8px',
      alignItems: 'center',
      marginBottom: '12px'
    });
    const left = document.createElement('div');
    Object.assign(left.style, { display: 'flex', gap: '6px', alignItems: 'center' });
    left.appendChild(this.navButton('Prev', () => {
      this.cursor = new Date(this.cursor.getFullYear(), this.cursor.getMonth() - 1, 1);
      this.render(body);
    }));
    const title = document.createElement('div');
    title.textContent = new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' }).format(this.cursor);
    Object.assign(title.style, { minWidth: '160px', textAlign: 'center', fontWeight: '800', color: '#0f172a' });
    left.appendChild(title);
    left.appendChild(this.navButton('Next', () => {
      this.cursor = new Date(this.cursor.getFullYear(), this.cursor.getMonth() + 1, 1);
      this.render(body);
    }));
    header.appendChild(left);

    const right = document.createElement('div');
    Object.assign(right.style, { display: 'flex', gap: '6px', alignItems: 'center' });
    right.appendChild(this.navButton('Today', () => {
      this.cursor = startOfMonth(new Date());
      this.scrollToToday();
      this.render(body);
    }));
    right.appendChild(this.navButton('Add Month to Pages', () => {
      this.addMonthPages();
      this.render(body);
    }, 'primary'));
    if (this.calendar) {
      right.appendChild(this.navButton('Sync Google', () => void this.syncGoogle(body)));
    }
    header.appendChild(right);
    body.appendChild(header);

    const grid = document.createElement('div');
    Object.assign(grid.style, {
      display: 'grid',
      gridTemplateColumns: 'repeat(7, minmax(0, 1fr))',
      gap: '6px'
    });
    body.appendChild(grid);

    for (const weekday of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']) {
      const cell = document.createElement('div');
      cell.textContent = weekday;
      Object.assign(cell.style, { fontSize: '12px', fontWeight: '800', color: '#475569', padding: '0 4px' });
      grid.appendChild(cell);
    }

    for (const day of monthGrid(this.cursor)) {
      const key = dateKey(day);
      const inMonth = day.getMonth() === this.cursor.getMonth();
      const dayEvents = events.filter((event) => event.dateKey === key);
      const cell = document.createElement('button');
      cell.type = 'button';
      Object.assign(cell.style, {
        minHeight: '92px',
        padding: '8px',
        textAlign: 'left',
        borderRadius: '8px',
        border: key === dateKey(new Date()) ? '2px solid #2563eb' : '1px solid #e2e8f0',
        background: inMonth ? '#fff' : '#f1f5f9',
        color: inMonth ? '#0f172a' : '#94a3b8',
        cursor: 'pointer',
        overflow: 'hidden'
      });
      const number = document.createElement('div');
      number.textContent = String(day.getDate());
      Object.assign(number.style, { fontWeight: '800', marginBottom: '6px' });
      cell.appendChild(number);
      for (const event of dayEvents.slice(0, 3)) {
        const item = document.createElement('div');
        item.textContent = `${event.time ? `${event.time} ` : ''}${event.title}`;
        Object.assign(item.style, {
          fontSize: '11px',
          lineHeight: '15px',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          color: '#0f766e'
        });
        cell.appendChild(item);
      }
      cell.addEventListener('click', () => this.openDay(body, key));
      grid.appendChild(cell);
    }
  }

  private openDay(body: HTMLElement, key: string): void {
    const events = this.loadEvents();
    const modal = makeOverlay(key);
    this.host.appendChild(modal.overlay);
    const form = document.createElement('form');
    Object.assign(form.style, { display: 'grid', gap: '8px', marginBottom: '12px' });
    const time = document.createElement('input');
    time.type = 'time';
    const title = document.createElement('input');
    title.placeholder = 'Event title';
    for (const input of [time, title]) {
      Object.assign(input.style, { height: '34px', border: '1px solid #cbd5e1', borderRadius: '7px', padding: '0 10px' });
    }
    const add = document.createElement('button');
    add.type = 'submit';
    add.textContent = 'Add Event';
    Object.assign(add.style, buttonStyle('primary'));
    form.append(time, title, add);
    form.addEventListener('submit', (event) => {
      event.preventDefault();
      if (!title.value.trim()) return;
      void this.createEvent(events, key, title.value.trim(), time.value, modal.close, body);
    });
    modal.body.appendChild(form);

    const dayEvents = events.filter((event) => event.dateKey === key);
    if (dayEvents.length === 0) modal.body.appendChild(smallText('No events.'));
    for (const event of dayEvents) {
      const row = document.createElement('div');
      Object.assign(row.style, {
        display: 'flex',
        justifyContent: 'space-between',
        gap: '8px',
        alignItems: 'center',
        padding: '8px',
        border: '1px solid #e2e8f0',
        borderRadius: '8px',
        background: '#fff',
        marginBottom: '8px'
      });
      row.appendChild(smallText(`${event.time || '--:--'} ${event.title}`));
      const del = document.createElement('button');
      del.type = 'button';
      del.textContent = 'Delete';
      Object.assign(del.style, buttonStyle('danger'));
      del.addEventListener('click', () => {
        void this.deleteEvent(events, event, modal.close, body);
      });
      row.appendChild(del);
      modal.body.appendChild(row);
    }
  }

  private addMonthPages(): void {
    const days = daysInMonth(this.cursor);
    for (const day of days) {
      const key = dateKey(day);
      const page = createPage({
        background: { kind: 'template', template: 'agenda' },
        sidePanel: { mode: 'day', dateKeys: [key] }
      });
      this.store.apply({ type: 'add-page', page: serializePage(page), index: this.store.doc.pageOrder.length });
    }
    this.store.apply({
      type: 'set-meta',
      meta: {
        calendarPageConfig: {
          mode: 'day',
          startDateKey: dateKey(days[0]!),
          startPage: this.store.doc.pageOrder.length - days.length,
          nextDateKey: dateKey(new Date(days[days.length - 1]!.getFullYear(), days[days.length - 1]!.getMonth(), days[days.length - 1]!.getDate() + 1))
        }
      }
    });
    this.renderer.rebuild();
  }

  private async syncGoogle(body: HTMLElement): Promise<void> {
    if (!this.calendar) return;
    try {
      const remote = await this.calendar.listMonth(this.cursor);
      const local = this.loadEvents().filter((event) => event.source !== 'google');
      this.saveEvents([
        ...local,
        ...remote.map((event) => ({ ...event, source: 'google' as const }))
      ]);
      this.render(body);
    } catch (err) {
      window.alert(err instanceof Error ? err.message : String(err));
    }
  }

  private async createEvent(
    events: CalendarEvent[],
    key: string,
    title: string,
    time: string,
    close: () => void,
    body: HTMLElement
  ): Promise<void> {
    try {
      if (this.calendar?.isSignedIn()) {
        const saved = await this.calendar.createEvent(key, title, time);
        events.push({ ...saved, source: 'google' });
      } else {
        events.push({ id: crypto.randomUUID(), dateKey: key, title, time, source: 'local' });
      }
      this.saveEvents(events);
      close();
      this.render(body);
    } catch (err) {
      window.alert(err instanceof Error ? err.message : String(err));
    }
  }

  private async deleteEvent(events: CalendarEvent[], event: CalendarEvent, close: () => void, body: HTMLElement): Promise<void> {
    try {
      if (event.source === 'google' && this.calendar?.isSignedIn()) {
        await this.calendar.deleteEvent(event.id);
      }
      this.saveEvents(events.filter((item) => item.id !== event.id));
      close();
      this.render(body);
    } catch (err) {
      window.alert(err instanceof Error ? err.message : String(err));
    }
  }

  private scrollToToday(): void {
    const today = dateKey(new Date());
    const pageId = this.store.doc.pageOrder.find((id) => {
      const page = this.store.doc.pages.get(id);
      return page?.sidePanel?.dateKeys.includes(today);
    });
    if (!pageId) return;
    const layout = this.renderer.layoutOf(pageId);
    if (!layout) return;
    this.renderer.setCamera({ ...this.renderer.camera, panY: -layout.y + 40 });
  }

  private navButton(label: string, run: () => void, kind: 'plain' | 'primary' = 'plain'): HTMLButtonElement {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = label;
    Object.assign(btn.style, buttonStyle(kind));
    btn.addEventListener('click', run);
    return btn;
  }

  private storageKey(): string {
    return `ihn_calendar_${this.store.doc.id}`;
  }

  private loadEvents(): CalendarEvent[] {
    try {
      const raw = localStorage.getItem(this.storageKey());
      return raw ? JSON.parse(raw) as CalendarEvent[] : [];
    } catch {
      return [];
    }
  }

  private saveEvents(events: CalendarEvent[]): void {
    localStorage.setItem(this.storageKey(), JSON.stringify(events));
  }
}

function startOfMonth(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), 1);
}

function dateKey(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

function monthGrid(cursor: Date): Date[] {
  const first = startOfMonth(cursor);
  const startOffset = (first.getDay() + 6) % 7;
  const start = new Date(first.getFullYear(), first.getMonth(), first.getDate() - startOffset);
  return Array.from({ length: 42 }, (_, i) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + i));
}

function daysInMonth(cursor: Date): Date[] {
  const out: Date[] = [];
  const first = startOfMonth(cursor);
  for (let d = new Date(first); d.getMonth() === first.getMonth(); d = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1)) {
    out.push(d);
  }
  return out;
}
