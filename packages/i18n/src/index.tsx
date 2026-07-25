import {
  createContext,
  createMemo,
  createSignal,
  useContext,
  type Accessor,
  type JSX,
} from "solid-js";
import { enUs, zhCn, type MessageKey } from "./messages";

export type AppLocale = "zh-CN" | "en-US";

interface I18nContextValue {
  locale: Accessor<AppLocale>;
  setLocale: (locale: AppLocale) => void;
  t: (key: MessageKey) => string;
}

const I18nContext = createContext<I18nContextValue>();

export interface I18nProviderProps {
  initialLocale?: AppLocale;
  onLocaleChange?: (locale: AppLocale) => void;
  children: JSX.Element;
}

export function I18nProvider(props: I18nProviderProps) {
  const [locale, setLocaleValue] = createSignal<AppLocale>(props.initialLocale ?? "zh-CN");
  const messages = createMemo(() => (locale() === "zh-CN" ? zhCn : enUs));
  const value: I18nContextValue = {
    locale,
    setLocale(nextLocale) {
      setLocaleValue(nextLocale);
      props.onLocaleChange?.(nextLocale);
    },
    t(key) {
      return messages()[key];
    },
  };
  return <I18nContext.Provider value={value}>{props.children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error("useI18n must be used inside I18nProvider");
  }
  return context;
}

export { enUs, zhCn, type MessageKey } from "./messages";
