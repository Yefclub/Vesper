import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  ReactNode,
} from "react";
import { api } from "./api";

type Catalog = Record<string, string>;

const I18nCtx = createContext<{
  locale: string;
  t: (key: string) => string;
  setLocale: (l: string) => void;
  catalog: Catalog;
}>({
  locale: "en",
  t: (k) => k,
  setLocale: () => {},
  catalog: {},
});

export function I18nProvider({
  initialLocale,
  children,
}: {
  initialLocale: string;
  children: ReactNode;
}) {
  const [locale, setLocale] = useState(initialLocale || "en");
  const [catalog, setCatalog] = useState<Catalog>({});

  useEffect(() => {
    api.i18nCatalog(locale).then(setCatalog).catch(() => setCatalog({}));
  }, [locale]);

  const t = useCallback(
    (key: string) => catalog[key] ?? key,
    [catalog],
  );

  const value = useMemo(
    () => ({ locale, t, setLocale, catalog }),
    [locale, t, catalog],
  );

  return <I18nCtx.Provider value={value}>{children}</I18nCtx.Provider>;
}

export function useI18n() {
  return useContext(I18nCtx);
}
