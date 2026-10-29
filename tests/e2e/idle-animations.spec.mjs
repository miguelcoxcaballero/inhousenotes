import { test, expect } from '@playwright/test';

// What keeps animating while the user does nothing. An infinite CSS animation
// runs for as long as its element is rendered, even at opacity 0, and every
// running animation makes the compositor (or, for paint properties, the main
// thread) produce a frame per vsync: heat on a phone with nothing to show.
const running = page => page.evaluate(() => document.getAnimations()
  .filter(animation => animation.playState === 'running')
  .map(animation => {
    const target = animation.effect?.target;
    return {
      name: animation.animationName || animation.transitionProperty || '',
      target: target ? `${target.id ? '#' + target.id : ''}.${[...target.classList].join('.')}${animation.effect.pseudoElement || ''}` : '',
      iterations: animation.effect.getTiming().iterations,
      properties: [...new Set(animation.effect.getKeyframes().flatMap(frame => Object.keys(frame)))]
        .filter(key => !['offset', 'computedOffset', 'easing', 'composite'].includes(key))
    };
  }));

// The always-present #loading-overlay spinner is visibility:hidden until
// shown; Chromium draws no frames for it (measured), so it is not counted.
const visible = animations => animations.filter(animation => !animation.target.startsWith('.spinner'));

test('Home at rest runs no animation, and pull to refresh spins only while it shows', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
  await page.evaluate(() => { showDriveHome(); stopDriveHomeAutoRefresh(); });
  await expect(page.locator('#drive-home')).toBeVisible();
  await page.waitForTimeout(400);
  expect(visible(await running(page))).toEqual([]);

  const spins = () => page.evaluate(() => document.getAnimations()
    .filter(animation => animation.animationName === 'home-pull-refresh-spin' && animation.playState === 'running').length);
  await page.evaluate(() => {
    const home = document.getElementById('drive-home');
    home.classList.add('home-is-pulling', 'home-is-refreshing');
    home.style.setProperty('--home-pull-opacity', '1');
  });
  await expect.poll(spins).toBe(1);
  const turning = await page.evaluate(() => {
    const spinner = document.querySelector('.home-pull-refresh-spinner');
    return { duration: getComputedStyle(spinner).animationDuration, count: getComputedStyle(spinner).animationIterationCount };
  });
  expect(turning).toEqual({ duration: '0.72s', count: 'infinite' });

  // Release: it keeps turning while it fades out, then stops.
  await page.evaluate(() => {
    const home = document.getElementById('drive-home');
    home.classList.remove('home-is-refreshing', 'home-is-pulling');
    home.classList.add('home-pull-resetting');
    home.style.setProperty('--home-pull-opacity', '0');
  });
  expect(await spins()).toBe(1);
  await page.evaluate(() => document.getElementById('drive-home').classList.remove('home-pull-resetting'));
  await expect.poll(spins).toBe(0);
  // Only the indicator's finite fade-out transitions may still be finishing.
  await expect.poll(async () => visible(await running(page))).toEqual([]);
});

test('loading placeholders slide on the compositor and stop by themselves', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
  await page.evaluate(() => {
    showDriveHome(); stopDriveHomeAutoRefresh();
    // The exact markup boot-v5.js writes while Drive is loading.
    const box = document.createElement('div');
    box.id = 'test-skeletons';
    box.innerHTML = '<div class="drive-card-skeleton" aria-hidden="true"><div class="skeleton-bar title"></div><div class="skeleton-preview"></div></div>'
      + '<div class="drive-folder-card-skeleton" aria-hidden="true"><div class="skeleton-icon"></div><div class="skeleton-bar"></div></div>'
      + '<span class="drive-status-skeleton" aria-hidden="true"></span>';
    document.querySelector('#drive-home .drive-main').prepend(box);
  });
  await page.waitForTimeout(200);
  const shimmers = (await running(page)).filter(animation => animation.name.startsWith('drive-skeleton'));
  expect(shimmers.length).toBe(5);
  for (const shimmer of shimmers) {
    expect(shimmer).toMatchObject({ name: 'drive-skeleton-slide', iterations: 25, properties: ['transform'] });
    expect(shimmer.target.endsWith('::after')).toBe(true);
  }
  // A placeholder looks the same: the gradient strip covers the whole box.
  const cover = await page.evaluate(() => {
    const bar = document.querySelector('.drive-status-skeleton');
    const box = bar.getBoundingClientRect();
    const strip = getComputedStyle(bar, '::after');
    return { width: box.width, overflow: getComputedStyle(bar).overflow, left: strip.left, right: strip.right, size: strip.backgroundSize };
  });
  expect(cover).toEqual({ width: cover.width, overflow: 'hidden', left: '-480px', right: '-240px', size: '480px 100%' });

  // Once the 30 s budget is used up nothing is left running.
  await page.evaluate(() => document.getAnimations().forEach(animation => {
    if (animation.animationName === 'drive-skeleton-slide') animation.currentTime = 31_000;
  }));
  expect((await running(page)).filter(animation => animation.name.startsWith('drive-skeleton'))).toEqual([]);
});

test('the peer search pulse animates only opacity and transform, for a bounded time', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
  await page.evaluate(() => __IHN_TEST_API__.resetLocalDocument(1, 'Pulse'));
  await page.evaluate(() => {
    // Same structure renderPresenceAvatars() builds for an online peer
    // without a direct link yet.
    const row = document.createElement('div');
    row.className = 'presence-avatars';
    row.style.cssText = 'position: fixed; top: 20px; left: 20px; z-index: 5000;';
    const wrapper = document.createElement('div');
    wrapper.className = 'presence-avatar-wrapper p2p-connecting';
    wrapper.innerHTML = '<img class="presence-avatar-img" alt="">';
    row.appendChild(wrapper);
    document.body.appendChild(row);
  });
  const pulse = (await running(page)).filter(animation => animation.name.startsWith('presence-peer-connecting'));
  expect(pulse.map(animation => animation.name).sort()).toEqual([
    'presence-peer-connecting', 'presence-peer-connecting-glow', 'presence-peer-connecting-scale'
  ]);
  for (const animation of pulse) {
    expect(animation.iterations).toBe(23);
    for (const property of animation.properties) expect(['opacity', 'transform']).toContain(property);
  }
  // The halo keeps the old shadow at its brightest and rests on the dim frame.
  const halo = await page.evaluate(() => {
    const wrapper = document.querySelector('.presence-avatar-wrapper.p2p-connecting');
    document.getAnimations().forEach(animation => { if (animation.effect.target === wrapper) { animation.pause(); animation.currentTime = 60_000; } });
    const after = getComputedStyle(wrapper, '::after'), before = getComputedStyle(wrapper, '::before');
    return { shadow: after.boxShadow, haloOpacity: after.opacity, ringOpacity: before.opacity };
  });
  expect(halo.shadow).toBe('rgba(66, 133, 244, 0.12) 0px 0px 0px 2px, rgba(66, 133, 244, 0.48) 0px 0px 11px 0px');
  expect(Number(halo.ringOpacity)).toBeCloseTo(0.25, 2);
  expect(Number(halo.haloOpacity)).toBeCloseTo(0.09375, 3);
});
