// Instellingen › Accounts: who you are on this PC (name, picture) and where to change it.

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { bridge } from '../../../core/bridge.js';
import { getShellInfo } from '../../../core/theme.js';
import { SettingsCard, SettingsGroup, Avatar, Dialog, Button } from '../../../core/components.js';
import { get } from '../common.js';
import { powerAction } from './power.js';

const t = makeT({
  nl: {
    local: 'Lokaal account',
    username: 'Gebruikersnaam: {u}',
    settings: 'Accountinstellingen',
    info: 'Je gegevens', infoDesc: 'Je naam en profielfoto',
    infoTitle: 'Naam en profielfoto wijzigen',
    infoBody: 'Win11 Shell gebruikt de naam en foto van je Ubuntu-account. Je wijzigt ze in de app Instellingen van Ubuntu: Systeem › Gebruikers. Daarna zie je de nieuwe naam en foto ook hier.',
    signin: 'Aanmeldingsopties', signinDesc: 'Wachtwoord wijzigen, automatisch aanmelden',
    signinTitle: 'Aanmelden en wachtwoord',
    signinBody: 'Je wachtwoord wijzig je in de app Instellingen van Ubuntu: Systeem › Gebruikers › Wachtwoord. Kies een wachtwoord dat je goed onthoudt; Win11 Shell bewaart nooit je wachtwoord.',
    lock: 'Vergrendelen', lockDesc: 'Het scherm vergrendelen als je even weggaat (Windows-logotoets + L)',
    lockBtn: 'Nu vergrendelen',
    privacy: 'Je account blijft op deze computer. Win11 Shell stuurt geen gegevens naar internet.',
  },
  en: {
    local: 'Local account',
    username: 'User name: {u}',
    settings: 'Account settings',
    info: 'Your info', infoDesc: 'Your name and profile picture',
    infoTitle: 'Change name and picture',
    infoBody: 'Win11 Shell uses the name and picture of your Ubuntu account. Change them in Ubuntu’s Settings app: System › Users. The new name and picture then appear here too.',
    signin: 'Sign-in options', signinDesc: 'Change password, automatic sign-in',
    signinTitle: 'Sign-in and password',
    signinBody: 'Change your password in Ubuntu’s Settings app: System › Users › Password. Choose a password you remember well; Win11 Shell never stores your password.',
    lock: 'Lock', lockDesc: 'Lock the screen when you step away (Windows logo key + L)',
    lockBtn: 'Lock now',
    privacy: 'Your account stays on this computer. Win11 Shell doesn’t send any data to the internet.',
  },
});

export const pages = {
  async accounts(ctx) {
    const info = await get('system.info');
    const sh = getShellInfo() || {};
    const u = info?.user || sh.user || {};
    const name = u.realName || u.name || '';
    const hero = h('section', { class: 'st-account-hero' },
      Avatar({ name, image: u.avatar ? bridge.fileUrl(u.avatar) : null, size: 96 }),
      h('div', { class: 'st-account-hero__text' },
        h('div', { class: 'st-account-hero__name' }, name),
        h('div', { class: 'st-account-hero__line' }, t('local')),
        u.name ? h('div', { class: 'st-account-hero__line' }, t('username', { u: u.name })) : null));
    return [
      hero,
      SettingsGroup({ title: t('settings'), children: [
        SettingsCard({ icon: 'user', title: t('info'), description: t('infoDesc'), onClick: () => Dialog.alert({ title: t('infoTitle'), message: t('infoBody') }) }),
        SettingsCard({ icon: 'key', title: t('signin'), description: t('signinDesc'), onClick: () => Dialog.alert({ title: t('signinTitle'), message: t('signinBody') }) }),
        SettingsCard({ icon: 'lock', title: t('lock'), description: t('lockDesc'), control: Button({ label: t('lockBtn'), onClick: () => powerAction({ action: 'lock', icon: 'lock', confirm: false }) }) }),
      ] }),
      SettingsGroup({ children: [SettingsCard({ icon: 'shield', title: t('privacy') })] }),
    ];
  },
};

