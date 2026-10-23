import { expect, test } from "@playwright/test";
import type { InputHTMLAttributes } from "react";
import { loadComponent } from "./fixtures/load-component";

interface Element {
  type: string;
  props: InputHTMLAttributes<HTMLInputElement>;
  children: Element[];
}

function mountHistoryLimit(settings: { saved?: number } = { saved: 50 }) {
  let draft: string | null = null;
  const writes: { key: string; value: number }[] = [];
  const { HistoryLimit } = loadComponent<{
    HistoryLimit: (props: Record<string, never>) => Element;
  }>(new URL("../src/components/settings/HistoryLimit.tsx", import.meta.url), {
    react: {
      createElement: (
        type: string,
        props: Element["props"],
        ...children: Element[]
      ) => ({ type, props, children }),
      useState: () => [draft, (value: string | null) => (draft = value)],
    },
    "react-i18next": { useTranslation: () => ({ t: (key: string) => key }) },
    "../../hooks/useSettings": {
      useSettings: () => ({
        getSetting: () => settings.saved,
        updateSetting: (key: string, value: number) => {
          writes.push({ key, value });
          settings.saved = value;
          return Promise.resolve(true);
        },
      }),
    },
    "../ui/Input": { Input: "input" },
    "../ui/SettingContainer": { SettingContainer: "container" },
  });
  const findInput = (element: Element): Element | undefined => {
    if (element?.type === "input") return element;
    return element?.children?.map(findInput).find(Boolean);
  };
  const input = () => findInput(HistoryLimit({}))!.props;
  const change = (value: string) => {
    input().onChange!({ target: { value } } as Parameters<
      NonNullable<Element["props"]["onChange"]>
    >[0]);
  };
  const commit = (
    valueAsNumber: number,
    valid: boolean,
    mode: "Enter" | "blur",
  ) => {
    let reports = 0;
    const currentTarget = {
      valueAsNumber,
      validity: { valid },
      reportValidity: () => {
        reports++;
        return valid;
      },
    } as HTMLInputElement;
    if (mode === "Enter") {
      input().onKeyDown!({ key: "Enter", currentTarget } as Parameters<
        NonNullable<Element["props"]["onKeyDown"]>
      >[0]);
    } else {
      input().onBlur!({ currentTarget } as Parameters<
        NonNullable<Element["props"]["onBlur"]>
      >[0]);
    }
    return reports;
  };
  return { writes, input, change, commit };
}

for (const mode of ["Enter", "blur"] as const) {
  for (const [draft, value] of [
    ["1e3", 1000],
    ["1E3", 1000],
    ["1e+3", 1000],
    ["1000", 1000],
    ["1", 1],
    ["5.0", 5],
  ] as const) {
    test(`${mode} commits the complete integer ${draft} as ${value}`, () => {
      const component = mountHistoryLimit();
      component.change(draft);
      expect(component.writes).toEqual([]);
      component.commit(value, true, mode);
      expect(component.writes).toEqual([{ key: "history_limit", value }]);
      // React compares number-input values loosely: a numeric prop would
      // leave equivalent text such as "1e3" in the DOM instead of "1000".
      expect(component.input().value).toBe(String(value));
    });
  }

  for (const [draft, value] of [
    ["0", 0],
    ["-1", -1],
    ["1001", 1001],
    ["1e4", 10000],
    ["1.5", 1.5],
    ["1e-1", 0.1],
    ["1e", NaN],
    ["12abc", NaN],
    ["Infinity", Infinity],
  ] as const) {
    test(`${mode} rejects ${draft} without a destructive write`, () => {
      const component = mountHistoryLimit();
      component.change(draft);
      const reports = component.commit(value, false, mode);
      expect(component.writes).toEqual([]);
      expect(reports).toBe(1);
      expect(component.input().value).toBe(draft);
    });
  }
}

test("empty edits restore the saved limit without writing", () => {
  const component = mountHistoryLimit();
  component.change("");
  component.commit(NaN, true, "Enter");
  expect(component.input().value).toBe("50");
  expect(component.writes).toEqual([]);
});

test("Escape cancels an invalid edit", () => {
  const component = mountHistoryLimit();
  component.change("0");
  component.input().onKeyDown!({ key: "Escape" } as Parameters<
    NonNullable<Element["props"]["onKeyDown"]>
  >[0]);
  expect(component.input().value).toBe("50");
  component.commit(50, true, "blur");
  expect(component.writes).toEqual([]);
});

test("an unchanged value does not write and the default remains five", () => {
  const component = mountHistoryLimit();
  component.change("50");
  component.commit(50, true, "Enter");
  expect(component.writes).toEqual([]);
  // Passing no persisted value exercises the production fallback.
  const missing = mountHistoryLimit({});
  expect(missing.input().value).toBe("5");
  expect(missing.input().min).toBe(1);
  expect(missing.input().max).toBe(1000);
});
