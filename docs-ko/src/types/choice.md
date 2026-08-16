# 선택

[합 타입](./either.md)에 관한 유명한 구호가 있다. ***잘못된 상태를 표현하지 못하게 하라***(Make illegal states unrepresentable)!

*선택(choice)* 타입(합 타입의 쌍대, 다른 말로는 *쌍대데이터*)에도 여기에 맞먹는 구호를 붙일 수 있다.

> ***잘못된 연산을 수행하지 못하게 하라!***

선택 타입은 Go나 Java 등의 *인터페이스*와 느슨하게 연관되어 있지만, 인터페이스와는 구분되는 중요한 특징이 있기 때문에 머리를 비우고 새로운 마음으로 배우는 것을 추천한다.

*선택 타입*은 유한 개의 분지(branch)로 이루어지며, 각각 다른 이름과 결과를 가진다.

선택 타입의 값은 그 값에 있는 분지 중 하나를 선택해 결과값을 얻는 개체이다.

```par
type ChooseStringOrNumber = choice {
  .string => String,
  .number => Int,
}
```

선택 타입은 키워드 `choice` 다음에 분지의 목록을 반점으로 구분하고 중괄호로 감싸서 작성한다.

각각의 분지는 온점으로 시작하고 소문자 이름을 가지며, `=>` 다음에 하나의 결과 타입을 필수적으로 작성해야 한다.

```par
choice {
  .branch1 => Result1,
  .branch2 => Result2,
  .branch3 => Result3,
}
```

결과 타입이 함수일 경우, 인자 부분을 화살표의 좌변으로 이동하고 소괄호로 감싸는 *문법 설탕* 역시 지원한다.

```par
type CancellableFunction<a, b> = choice {
  .cancel* => !,
  //.apply => [a] b,
  .apply(a) => b,
}
```

[함수](./function.md)와 같이 **선택 타입도 [선형](../types_and_expressions.md#선형성)이다.** 선택 값은 복사할 수 없으며, 주어진 분지 중 하나를 사용해 반드시 한 번 소멸시켜야 한다.

선택 값은 보통 명시적으로 소멸시켜야 하지만, 한 가지 유용한 예외가 있다. 선택 값은 분지 중 하나를 위의 `.cancel*`과 같이 별표(`*`)로 표시할 수 있다. 이렇게 하면 이 분지를 정리 연산으로 선언하게 되며, 분지의 결과 역시 정리가 가능하다면 선택 값 전체를 사용하지 않고 버려도 Par에서 자동으로 이 분지를 선택한다. 자세한 원리는 [자동 정리](./auto_cleanup.md) 장에서 다룬다.

> 선택 타입은 [반복](./iterative.md) 타입과 조합해 여러 번 조작할 수 있는 객체를 만드는 데 자주 쓰인다. 예를 들어, `@basic/Console`에 내장되어 표준 출력으로 출력하는 핸들로 쓰이는 `Console` 타입이 *반복 선택* 타입이다.
> 
> ```par
> type Console = iterative choice {
>   .close* => !,
>   .print(String) => self,
> }
> ```
> 
> 이 값을 사용해 여러 줄을 순서대로 출력할 수 있다.
> 
> ```par
> module Main
>
> import @basic/Console
>
> def Main = Console.Open
>   .print("First line.")
>   .print("Second line.")
>   .print("Third line.")
>   .close
> ```

## 생성

선택 타입의 값은 `case`식을 단독으로 작성해 생성한다.

```par
def Example: ChooseStringOrNumber = case {
  .string => "Hello!",
  .number => 42,
}
```

중괄호 안의 각 분지는 타입의 분지와 같은 문법으로 작성하되, 타입 대신 값을 작성하면 된다.

```par
def FormatInt: CancellableFunction<Int, String> = case {
  .cancel* => !,
  .apply(n) => `#{n}`,
}
```

[*분기* 타입](./either.md)의 `.case` 분지에 달리는 패턴과 달리, 선택 타입의 `case`식 분지는 결과값을 생성하는 역할을 하기 때문에 페이로드를 대입하지 않는다. 그 대신 화살표의 좌변에 함수 인자를 대입하는 것은 가능하다.

## 소멸

선택 값은 분지 중 하나를 *선택*해 결과값으로 바꿈으로써 소멸시킬 수 있다. 선택 타입의 값 뒤에 `.branch`를 적용하면 된다.

```par
def Number = Example.number  // = 42
```

위에서는 `CancellableFunction<a, b>` 타입과 그 타입을 가지는 값인 `FormatInt`을 정의했다. [일반 함수](./function.md)는 선형이므로 반드시 호출해야 하지만, 이렇게 정의한 *취소 가능한 함수*는 호출할지 말지를 선택할 수 있다.

이 타입을 사용해 옵션 값에 대한 *map* 함수를 정의할 수 있다.

```par
type Option<a> = either {
  .none!,
  .some a,
}

dec MapOption :
  [type a, type b, Option<a>, CancellableFunction<a, b>]
  Option<b>

def MapOption = [type a, type b, option, func] option.case {
  .none! => let ! = func.cancel in .none!,
//                  \_________/
  .some x => let y = func.apply(x) in .some y,
//                   \________/
}

def Result = MapOption(type Int, type String, .some 42, FormatInt)  // = .some "42"
```

이 예제를 보면 Par에서는 타입을 작성할 때 걱정 없이 여러 줄에 걸쳐 작성할 수 있음을 알 수 있다. Par의 문법 자체가 이런 스타일에 최적화되어 있다.
