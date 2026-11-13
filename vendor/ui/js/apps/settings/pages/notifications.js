// Instellingen › Systeem › Meldingen: toasts, sounds, Niet storen, test notification.

import { makeT } from '../../../core/i18n.js';
import { getSetting, setSetting } from '../../../core/theme.js';
import { SettingsCard, SettingsGroup, SettingsExpander, Checkbox, Button, Toast } from '../../../core/components.js';
import { call, settingToggle, toggleCard, notice, showError } from '../common.js';

const t = makeT({
  nl: {
    notifications: 'Meldingen',
    notificationsDesc: 'Meldingen ontvangen van apps en andere afzenders',
    sounds: 'Meldingen mogen een geluid afspelen',
    dnd: 'Niet storen',
    dndDesc: 'Meldingen worden stil bewaard in het meldingencentrum',
    test: 'Testmelding versturen',
    testDesc: 'Bekijk hoe een melding eruitziet',
    send: 'Versturen',
    testTitle: 'Testmelding',
    testBody: 'Zo ziet een melding eruit.',
    sent: 'Testmelding verstuurd',
    gnome: 'Onder GNOME toont GNOME zelf de meldingen. Deze instellingen gelden voor Win11 Shell.',
    dndOnNote: 'Niet storen staat aan: je ziet geen meldingen in beeld.',
  },
  en: {
    notifications: 'Notifications',
    notificationsDesc: 'Get notifications from apps and other senders',
    sounds: 'Allow notifications to play sounds',
    dnd: 'Do not disturb',
    dndDesc: 'Notifications are kept quietly in the notification centre',
    test: 'Send a test notification',
    testDesc: 'See what a notification looks like',
    send: 'Send',
    testTitle: 'Test notification',
    testBody: 'This is what a notification looks like.',
    sent: 'Test notification sent',
    gnome: 'In GNOME, notifications are shown by GNOME itself. These settings apply to Win11 Shell.',
    dndOnNote: 'Do not disturb is on: notifications won’t pop up.',
  },
});

export const pages = {
  async notifications(ctx) {
    const lst = await call('notifications.list');
    const supported = lst.ok ? lst.value?.supported !== false : true;

    const soundsBox = Checkbox({
      label: t('sounds'), checked: !!getSetting('notifications.sounds', true),
      onChange: async (on) => { try { await setSetting('notifications.sounds', on); } catch (e) { soundsBox.setChecked(!on); showError(e); } },
    });
    const exp = SettingsExpander({
      icon: 'notification', title: t('notifications'), description: t('notificationsDesc'),
      control: settingToggle(ctx, 'notifications.showToasts', { ariaLabel: t('notifications') }),
      expanded: true, children: [soundsBox],
    });

    const dndCard = toggleCard(ctx, { icon: 'notification-off', title: t('dnd'), description: t('dndDesc'), path: 'notifications.dnd' });
    const dndNote = notice({ message: t('dndOnNote') });
    const syncNote = () => { dndNote.hidden = !getSetting('notifications.dnd', false); };
    syncNote();
    ctx.onSettings(() => { syncNote(); soundsBox.setChecked(!!getSetting('notifications.sounds', true)); });

    const sendBtn = Button({
      label: t('send'),
      onClick: async () => {
        sendBtn.disabled = true;
        const r = await call('notifications.test');
        sendBtn.disabled = false;
        if (r.ok) Toast.show({ severity: 'success', title: t('sent'), timeout: 2500 });
        else Toast.show({ icon: 'notification', title: t('testTitle'), message: t('testBody'), timeout: 4000 });
      },
    });

    return [
      supported ? null : notice({ message: t('gnome') }),
      SettingsGroup({ children: [exp, dndCard, dndNote] }),
      SettingsGroup({ children: [SettingsCard({ icon: 'play', title: t('test'), description: t('testDesc'), control: sendBtn })] }),
    ].filter(Boolean);
  },
};
