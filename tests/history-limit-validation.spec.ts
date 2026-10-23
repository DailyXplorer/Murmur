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
  const commit = (valueAsNumber: number, mode: "Enter" | "blur") => {
    let reports = 0;
    const currentTarget = {
      valueAsNumber,
      reportValidity: () => {
        reports++;
        return false;
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
      component.commit(value, mode);
      expect(component.writes).toEqual([{ key: "history_limit", value }]);
      // React compares number-input values loosely: a numeric prop would
      // leave equivalent text such as "1e3" in the DOM instead of "1000".
      expect(component.input().value).toBe(String(value));
    });
  }

  for (const [draft, value, limit] of [
    ["0", 0, 1],
    ["-1", -1, 1],
    ["1001", 1001, 1000],
    ["1e4", 10000, 1000],
  ] as const) {
    test(`${mode} saves ${draft} as ${limit}`, () => {
      const component = mountHistoryLimit();
      component.change(draft);
      component.commit(value, mode);
      expect(component.writes).toEqual([
        { key: "history_limit", value: limit },
      ]);
      expect(component.input().value).toBe(String(limit));
    });
  }

  for (const [draft, value] of [
    ["1.5", 1.5],
    ["1e-1", 0.1],
    ["1e", NaN],
    ["12abc", NaN],
    ["Infinity", Infinity],
  ] as const) {
    test(`${mode} drops ${draft} and shows the saved limit again`, () => {
      const component = mountHistoryLimit();
      component.change(draft);
      const reports = component.commit(value, mode);
      expect(component.writes).toEqual([]);
      expect(reports).toBe(0);
      expect(component.input().value).toBe("50");
    });
  }
}

test("empty edits restore the saved limit without writing", () => {
  const component = mountHistoryLimit();
  component.change("");
  component.commit(NaN, "Enter");
  expect(component.input().value).toBe("50");
  expect(component.writes).toEqual([]);
});

test("Escape cancels an edit", () => {
  const component = mountHistoryLimit();
  component.change("0");
  component.input().onKeyDown!({ key: "Escape" } as Parameters<
    NonNullable<Element["props"]["onKeyDown"]>
  >[0]);
  expect(component.input().value).toBe("50");
  component.commit(50, "blur");
  expect(component.writes).toEqual([]);
});

test("an unchanged value does not write and the default remains five", () => {
  const component = mountHistoryLimit();
  component.change("50");
  component.commit(50, "Enter");
  expect(component.writes).toEqual([]);
  // Passing no persisted value exercises the production fallback.
  const missing = mountHistoryLimit({});
  expect(missing.input().value).toBe("5");
  expect(missing.input().min).toBe(1);
  expect(missing.input().max).toBe(1000);
});
