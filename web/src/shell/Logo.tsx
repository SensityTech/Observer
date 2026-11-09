import { Mark } from '../components/Mark';
import { useI18n } from '../i18n/I18nProvider';

export function Logo() {
    const { t } = useI18n();
    return (
        <span className="logo">
            <Mark size={24} />
            {t.app.name}
        </span>
    );
}
