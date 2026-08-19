import { getCurrentWindow } from '@tauri-apps/api/window';
import { driver, type DriveStep, type Driver, type PopoverDOM } from 'driver.js';
import { nextTick, onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue';

export const ONBOARDING_COMPLETED_STORAGE_KEY = 'printbridge.onboarding.completed';

type OnboardingTab =
  | 'settings'
  | 'remote'
  | 'website-whitelist'
  | 'ip-whitelist'
  | 'logs'
  | 'updates';

type Translate = (key: string) => string;

interface OnboardingStep {
  selector: string;
  tab: OnboardingTab;
  titleKey: string;
  descriptionKey: string;
}

interface UseOnboardingOptions {
  activeTab: Ref<string>;
  ready: Readonly<Ref<boolean>>;
  t: Translate;
}

interface OnboardingController {
  replayOnboarding: () => Promise<void>;
}

const ONBOARDING_STEPS: OnboardingStep[] = [
  {
    selector: '[data-tour="app-status"]',
    tab: 'settings',
    titleKey: 'onboardingStatusTitle',
    descriptionKey: 'onboardingStatusDescription',
  },
  {
    selector: '[data-tour="print-settings"]',
    tab: 'settings',
    titleKey: 'onboardingPrintingTitle',
    descriptionKey: 'onboardingPrintingDescription',
  },
  {
    selector: '[data-tour="service-settings"]',
    tab: 'settings',
    titleKey: 'onboardingServiceTitle',
    descriptionKey: 'onboardingServiceDescription',
  },
  {
    selector: '[data-tour="preferences-cli"]',
    tab: 'settings',
    titleKey: 'onboardingPreferencesTitle',
    descriptionKey: 'onboardingPreferencesDescription',
  },
  {
    selector: '[data-tour="config-transfer"]',
    tab: 'settings',
    titleKey: 'onboardingTransferTitle',
    descriptionKey: 'onboardingTransferDescription',
  },
  {
    selector: '[data-tour="remote-settings"]',
    tab: 'remote',
    titleKey: 'onboardingRemoteTitle',
    descriptionKey: 'onboardingRemoteDescription',
  },
  {
    selector: '[data-tour="website-whitelist"]',
    tab: 'website-whitelist',
    titleKey: 'onboardingWebsiteTitle',
    descriptionKey: 'onboardingWebsiteDescription',
  },
  {
    selector: '[data-tour="ip-whitelist"]',
    tab: 'ip-whitelist',
    titleKey: 'onboardingIpTitle',
    descriptionKey: 'onboardingIpDescription',
  },
  {
    selector: '[data-tour="task-history"]',
    tab: 'logs',
    titleKey: 'onboardingTasksTitle',
    descriptionKey: 'onboardingTasksDescription',
  },
  {
    selector: '[data-tour="about"]',
    tab: 'updates',
    titleKey: 'onboardingAboutTitle',
    descriptionKey: 'onboardingAboutDescription',
  },
];

/** 判断当前 WebView 是否已经完成或跳过首次使用引导。 */
export function isOnboardingCompleted(storage: Storage = window.localStorage): boolean {
  return storage.getItem(ONBOARDING_COMPLETED_STORAGE_KEY) === 'true';
}

/** 记录当前 WebView 已经完成或跳过首次使用引导。 */
export function markOnboardingCompleted(storage: Storage = window.localStorage): void {
  storage.setItem(ONBOARDING_COMPLETED_STORAGE_KEY, 'true');
}

/** 等待跨 Tab 后的目标元素渲染，超时后允许跳过当前步骤。 */
async function waitForElement(selector: string, timeoutMs = 500): Promise<boolean> {
  if (document.querySelector(selector)) return true;

  return new Promise((resolve) => {
    const observer = new MutationObserver(() => {
      if (!document.querySelector(selector)) return;
      observer.disconnect();
      window.clearTimeout(timeoutId);
      resolve(true);
    });
    const timeoutId = window.setTimeout(() => {
      observer.disconnect();
      resolve(Boolean(document.querySelector(selector)));
    }, timeoutMs);

    observer.observe(document.documentElement, { childList: true, subtree: true });
  });
}

/** 在 Driver.js 弹层中加入唯一的显式退出操作。 */
function renderSkipButton(popover: PopoverDOM, label: string, onSkip: () => void): void {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'driver-popover-footer-btn printbridge-tour-skip-btn';
  button.textContent = label;
  button.addEventListener('click', onSkip, { once: true });
  popover.footerButtons.prepend(button);
}

/** 管理桌面窗口首次可见门控、Driver.js 生命周期和手动重播。 */
export function useOnboarding(options: UseOnboardingOptions): OnboardingController {
  const windowHasBeenVisible = ref(false);
  let autoStartAttempted = false;
  let activeTour: Driver | null = null;
  let unlistenFocus: (() => void) | null = null;

  async function activateTab(tab: OnboardingTab): Promise<void> {
    options.activeTab.value = tab;
    await nextTick();
  }

  async function startOnboarding(): Promise<void> {
    if (activeTour) return;

    const startingTab = options.activeTab.value;
    const steps: DriveStep[] = ONBOARDING_STEPS.map((step) => ({
      element: step.selector,
      popover: {
        title: options.t(step.titleKey),
        description: options.t(step.descriptionKey),
      },
    }));
    let tourStarted = false;
    let cleanedUp = false;

    const cleanup = (): void => {
      if (cleanedUp) return;
      cleanedUp = true;
      if (activeTour === tour) activeTour = null;
      void activateTab(startingTab as OnboardingTab);
    };

    const finish = (completed: boolean): void => {
      if (cleanedUp) return;
      if (completed) markOnboardingCompleted();
      tour.destroy();
      cleanup();
    };

    const showStep = async (index: number, direction: 1 | -1): Promise<void> => {
      let candidate = index;

      while (candidate >= 0 && candidate < ONBOARDING_STEPS.length) {
        const step = ONBOARDING_STEPS[candidate];
        await activateTab(step.tab);
        if (activeTour !== tour) return;
        if (await waitForElement(step.selector)) {
          if (activeTour !== tour) return;
          if (tourStarted) {
            tour.moveTo(candidate);
          } else {
            tourStarted = true;
            tour.drive(candidate);
          }
          return;
        }
        candidate += direction;
      }

      finish(tourStarted && direction === 1);
    };

    const tour = driver({
      steps,
      allowClose: false,
      allowKeyboardControl: false,
      disableActiveInteraction: true,
      overlayOpacity: 0.62,
      popoverClass: 'printbridge-tour-popover',
      showButtons: ['previous', 'next'],
      showProgress: true,
      progressText: '{{current}} / {{total}}',
      prevBtnText: options.t('onboardingPrevious'),
      nextBtnText: options.t('onboardingNext'),
      doneBtnText: options.t('onboardingDone'),
      onNextClick: (_element, _step, { index }) => {
        void showStep((index ?? -1) + 1, 1);
      },
      onPrevClick: (_element, _step, { index }) => {
        void showStep((index ?? 1) - 1, -1);
      },
      onDoneClick: () => finish(true),
      onPopoverRender: (popover) => {
        renderSkipButton(popover, options.t('onboardingSkip'), () => finish(true));
      },
      onDestroyed: cleanup,
    });

    activeTour = tour;
    await showStep(0, 1);
  }

  async function updateWindowVisibility(): Promise<void> {
    const currentWindow = getCurrentWindow();
    const [visible, focused] = await Promise.all([
      currentWindow.isVisible(),
      currentWindow.isFocused(),
    ]);
    if (visible && focused) windowHasBeenVisible.value = true;
  }

  async function setupWindowTracking(): Promise<void> {
    const currentWindow = getCurrentWindow();
    unlistenFocus = await currentWindow.onFocusChanged(({ payload: focused }) => {
      if (focused) void updateWindowVisibility();
    });
    await updateWindowVisibility();
  }

  watch([options.ready, windowHasBeenVisible], ([ready, visible]) => {
    if (!ready || !visible || autoStartAttempted || isOnboardingCompleted()) return;
    autoStartAttempted = true;
    void startOnboarding();
  });

  onMounted(() => {
    void setupWindowTracking();
  });

  onBeforeUnmount(() => {
    unlistenFocus?.();
    const tour = activeTour;
    activeTour = null;
    tour?.destroy();
  });

  return {
    replayOnboarding: startOnboarding,
  };
}
