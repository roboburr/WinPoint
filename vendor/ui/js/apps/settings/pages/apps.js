// Instellingen › Apps (overview).

import { SettingsGroup } from '../../../core/components.js';
import { linkCard } from '../common.js';

export const pages = {
  async apps(ctx) {
    return [SettingsGroup({ children: ['installed', 'defaultapps', 'startup'].map((id) => linkCard(ctx, id)) })];
  },
};
