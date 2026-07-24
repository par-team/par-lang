# 분기

*분기(either) 타입*은 유명한 [합 타입](https://en.wikipedia.org/wiki/Tagged_union), 다른 용어로는 <em>태그 공용체(tagged union)</em>에 해당한다.

분기 타입은 유한한 개수의 선지(variant)를 정의하며, 각각 다른 이름과 페이로드를 가진다. 분기 타입의 값은 그 타입의 선지 중 하나에 해당한다.

```par
module Main

import {
  @core/Int
  @core/String
}

type StringOrNumber = either {
  .string String,
  .number Int,
}

def Str: StringOrNumber = .string "Hello!"
def Num: StringOrNumber = .number 42
```

분기 타입은 키워드 `either` 다음에 선지의 목록을 반점으로 구분하고 중괄호로 감싸서 작성한다.

각각의 선지는 온점으로 시작하고 소문자 이름을 가지며, 하나의 페이로드 타입을 필수적으로 작성해야 한다.

```par
either {
  .variant1 Payload1,
  .variant2 Payload2,
  .variant3 Payload3,
}
```

각 페이로드는 단일 타입이어야 하므로, 합성 페이로드를 정의할 때는 [단위](./unit.md)나 [순서쌍](./pair.md) 등의 여러 가지 타입을 사용한다. 예를 들어 보자.

```par
type MaybeBoth<a, b> = either {
  .neither!,
  .left a,
  .right b,
  .both(a, b)!,
}
```

> 분기 타입은 [*재귀*](./recursive.md) 타입과 같이 사용해 유한한 트리 형태의 구조를 정의하는 데 쓰이는 경우가 많다.
>
> ```par
> type BinaryTree<a> = recursive either {
>   .empty!,
>   .node(a, self, self)!,
> }
> ```
>
> 내장된 `List<a>` 타입 역시 재귀와 분기 타입의 조합으로 정의되어 있다.
>
> ```par
> type List<a> = recursive either {
>   .end!,
>   .item(a) self,
> }
> ```

## 생성

분기 타입의 값은 그 타입의 선지 중 하나의 이름을 사용해 `.name`으로 시작하고, 이에 대응하는 페이로드 타입의 값을 덧붙여서 생성한다.

아래 예제 코드에서 분기 타입이 가질 수 있는 여러 가지 페이로드를 확인할 수 있다.

```par
type Varied = either {
  .unit!,                        // 페이로드는 `!`
  .string String,                // 페이로드는 `String`
  .number Int,                   // 페이로드는 `Int`
  .pair(Int) String,             // 페이로드는 `(Int) String`
  .symmetricPair(Int, String)!,  // 페이로드는 `(Int, String)!`
  .nested either {               // 페이로드는 다른 분기 타입
    .left!,
    .right!,
  },
  .nested2(String) either {      // 페이로드는 `String`과 다른 분기의 순서쌍
    .left!,
    .right!,
  }
}

def Example1: Varied = .unit!
def Example2: Varied = .string "Hello!"
def Example3: Varied = .number 42
def Example4: Varied = .pair(42) "Hello!"
def Example5: Varied = .symmetricPair(42, "Hello!")!
def Example6: Varied = .nested.left!
def Example7: Varied = .nested.right!
def Example8: Varied = .nested2("Hello!").left!
def Example9: Varied = .nested2("Hello!").right!
```

분기 타입의 페이로드로는 [순서쌍](./pair.md) 타입이 자주 쓰이며, 대칭 스타일과 순차 스타일 모두 자주 쓰인다. 순차 스타일로 사용할 경우 위의 `.nested2` 예시와 같이 페이로드에 다른 분기 타입을 자연스럽게 연쇄할 수 있다.

## 소멸

분기 타입의 값은 다른 언어에서의 패턴 매칭과 같이 `.case`식으로 소멸시킬 수 있다.

`.case`식은 소멸시킬 값으로 시작하고, `.case` 다음에 각 선지마다 하나씩의 분지(branch)를 반점으로 구분하고 중괄호로 감싸서 작성한다.

```par
value.case {
  // 분지 목록
}
```

각 분지는 해당하는 선지의 이름으로 시작하고, 페이로드를 대입할 *패턴*, `=>`, 그 분지의 결과를 계산하는 식의 순서로 작성한다. 모든 분지는 같은 타입을 가져야 한다.

```par
// 단일 분지
.name pattern => expression,
```

페이로드가 대입되는 패턴은 `let`식의 좌변에 올 수 있는 패턴과 같다.
- 변수(`variable`) 패턴은 값 전체를 매치한다.
- `!`는 [단위](./unit.md) 값을 매치한다.
- `(pattern1, ...) patternN`은 [순서쌍](./pair.md)을 매치한다.

위에서 작성한 `StringOrNumber` 타입의 `Str`나 `Num` 값을 분석하는 예시를 살펴보자.

```par
// "Hello!"로 평가됨
def ResultForStr = Str.case {
  .string s => s,
  .number n => `#{n}`,
}

// "42"로 평가됨
def ResultForNum = Num.case {
  .string s => s,
  .number n => `#{n}`,
}
```

더 다양한 예시가 필요하다면 위의 `Varied` 타입을 `String`으로 변환하는 아래 [함수](./function.md)를 참고하면 된다.

```par
dec VariedToString : [Varied] String
def VariedToString = [varied] varied.case {
  .unit! => ".unit!",

  .string s => String.Builder.add(".string ").add(String.Quote(s)).build,

  .number n => `.number #{n}`,

  .pair(n) s =>
    `.pair(#{n}) ${String.Quote(s)}`,

  .symmetricPair(n, s)! =>
    `.symmetricPair(#{n}, ${String.Quote(s)})!`,

  .nested inside => String.Builder.add(".nested").add(inside.case {
    .left! => ".left!",
    .right! => ".right!",
  }).build,

  .nested2(s) inside =>
    String.Builder
      .add(".nested2(")
      .add(String.Quote(s))
      .add(")")
      .add(inside.case {
        .left! => ".left!",
        .right! => ".right!",
      }).build,
}
```
