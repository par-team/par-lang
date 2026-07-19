# 선택과 송신

내장 `String.Builder` 타입은 다음과 같이 정의되어 있다.

```par
type String.Builder = iterative choice {
  .add(String) => self,
  .build => String,
}
```

`.add`와 `.build`의 두 메서드가 있는 OOP 스타일의 객체라고 생각할 수 있다. 가장 바깥쪽에서 보면 [반복](../../types/iterative.md) [선택](../../types/choice.md), 즉 연거푸 상호작용할 수 있는 객체에 해당한다.

`String.Builder`를 생성할 때는 같은 이름의 내장 정의를 사용할 수 있다.

```par
module Main

import @core/String

def LetsBuildStrings = do {
  let builder = String.Builder
  // 이하 코드 생략
```

## 선택

이제 `String.Builder` 타입의 지역 변수 `builder`를 만들었다.

[반복](../../types/iterative.md) 타입을 배우면서 반복 값은 해당하는 타입의 본문처럼 취급하면 된다는 것도 배운 바가 있다. `String.Builder`의 경우에는 **선택 타입**이며, 이에 대응하는 명령은 **선택 명령**이다.

모든 명령은 주어로 시작하며, 여기서는 `builder` 변수가 주어가 된다. 선택 명령 자체는 보통의 [선택 값 소멸](../../types/choice.md#소멸)과 같은 형태로 이루어져 있다.

```par
def LetsBuildStrings = do {
  let builder = String.Builder
  builder.add        // 선택 명령
  // 이하 코드 생략
```

간단하다! 이제부터가 중요하다. 선택 명령을 실행한 뒤에는 `builder`가 선택된 분지의 결과로 **타입을 바꾼다**. 지금 선택한 분지는 다음과 같다.

```par
  .add(String) => self,
```

`=>` 좌변의 인자는 [함수](../../types/function.md) 타입의 문법 설탕으로, 원래 문법은 다음과 같다.

```par
  .add => [String] self,
```

즉, `builder.add` 명령을 수행하고 난 뒤에 `builder`의 타입은 함수가 된다.

```par
builder: [String] iterative choice {
  .add(String) => self,
  .build => String,
}
```

> 원래 분지의 `self`는 대응하는 `iterative`로 치환되었다. 여기서는 원래의 `String.Builder`에 해당한다.

## 송신

**함수 타입**의 경우에는 **송신 명령**이 있다. 함수 호출과 같은 모양이지만 명령이므로 결과를 반환하지 않고, 그 대신 주어 자체가 결과값이 된다.

```par
def LetsBuildStrings = do {
  let builder = String.Builder
  builder.add        // 선택 명령
  builder("Hello")   // 송신 명령
```

> 위의 코드를 보고 **선택**과 **송신** 명령이 변수를 일반적인 방법으로 소멸시키고 재대입하는 것과 같은 동작을 하는 것을 눈치챘을 수도 있다. 코드로 보면...
>
> ```par
>   let builder = builder.add
>   let builder = builder("Hello")
> ```
>
> 이 두 명령의 경우에는 완전히 같은 동작을 하는 것이 맞으며, 이 명령들이 무슨 동작을 하는지 직관을 쌓는 데도 도움이 된다. 하지만 `.case`와 *수신* 명령을 배우기 시작하면 위의 간단한 대응 관계가 더 이상 성립하지 않는다.

문자열을 송신하고 난 뒤에는 `builder`가 원래의 `String.Builder`로 돌아가므로 내용을 계속 추가할 수 있다.

```par
def LetsBuildStrings = do {
  let builder = String.Builder
  builder.add        // 선택 명령
  builder("Hello")   // 송신 명령
  builder.add        // 선택 명령
  builder(", ")      // 송신 명령
  builder.add        // 선택 명령
  builder("World")   // 송신 명령
  builder.add        // 선택 명령
  builder("!")       // 송신 명령
```

## 명령의 연쇄

코드가 다소 번잡하지만, 다행히 개선의 여지가 있다! **주어가 같은 명령**이 여러 개 연속될 경우에는 주어를 매번 반복하지 않고 **연쇄하여 작성**할 수 있다.

```par
module Main

import @core/String

def LetsBuildStrings = do {
  let builder = String.Builder
  builder.add("Hello")
  builder.add(", ")
  builder.add("World")
  builder.add("!")
```

더 줄일 수도 있다.

```par
def LetsBuildStrings = do {
  let builder = String.Builder
  builder
    .add("Hello")
    .add(", ")
    .add("World")
    .add("!")
```

개인적으로는 더 줄이기 전의 전자가 마음에 든다.

지금까지 생성한 문자열을 반환해서 `do`식을 마무리하자.

```par
def LetsBuildStrings = do {
  let builder = String.Builder
  builder.add("Hello")
  builder.add(", ")
  builder.add("World")
  builder.add("!")
} in builder.build  // = "Hello, world!"
```
