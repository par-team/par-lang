# 전칭

*제네릭* 함수는 어떻게 만들 수 있을까? 제네릭 값은?

[제네릭 타입](../structure/definitions_and_declarations.md)은 이미 배운 바가 있다. 예를 들어, 다른 언어에서 볼 수 있고 Par에서도 내장 타입으로 지원하는 일반적인 옵션 타입은 다음과 같이 작성할 수 있다.

```par
type Option<a> = either {
  .none!,
  .some a,
}
```

Par의 제네릭 타입 정의도 익숙한 부등호 문법을 사용한다. `Option<a>`의 `a`와 같은 제네릭 타입의 인자는 `Option<Int>` 등과 같이 다른 어떤 타입이든 대입할 수 있지만, 이렇게 생성되는 타입은 항상 구상(concrete) 타입이다.

이제 아래의 두 정의를 확인해 보자.

```par
module Main

import {
  @core/Int
  @core/String
  @core/Option
}

def None: Option<String> = .none!

dec Swap : [(String, Int)!] (Int, String)!
def Swap = [pair]
  let (first, second)! = pair
  in (second, first)!
```

둘 모두 구상 타입을 이용해 정의했지만, 그 구상 타입을 실제로 사용하지는 않는다. `.none!`은 어떤 `Option<a>`에 대해서도 유효한 값이며, 순서쌍의 원소를 뒤집는 것은 순서쌍의 내용에 상관 없이 가능하다.

이 함수를 타입에 상관 없이 사용하고 싶다면, *전칭(forall)* 타입을 사용하면 된다!

Par의 **전칭 타입**은...
- **부등호를 사용하지 않는다.** 그보다는 타입을 입력받는 함수에 더 가깝다.
- **추론되지 않는다.** 제네릭 함수를 호출하려면 타입을 명시적으로 작성해야 한다.
- **일급이다!** 제네릭 값을 원하는 대로 보관하고 이동시켜도 제네릭이 유지된다.

사용할 때 타입이 추론되는 제네릭이 필요하다면, [암시적 제네릭](./implicit_generics.md)을 사용하면 된다.

전칭 타입은 다음과 같이 **두 부분으로 구성된다.**
- 대괄호 안에 소문자 타입 변수를 적고 키워드 `type`을 앞에 붙인다.
- 바로 뒤에 이 타입 변수를 사용하는 결과 타입을 작성한다.

```par
dec None : [type a] Option<a>

dec Swap : [type a, type b, (a, b)!] (b, a)!
```

위에 작성했던 `None`의 구상 버전 정의를 지우고 나면 위의 선언이 그대로 제네릭 타입이 된다. 코드에서 볼 수 있듯이 함수인데 타입을 인자로 받는다고 해도 과언이 아니다!

위의 `Swap`과 같이 전칭 타입에 다른 명시적 타입 대입자나 값 인자가 따라올 경우 같은 대괄호 안에 묶어서 작성할 수 있으며, `type`은 명시적 타입 대입자마다 반복 작성한다.

```par
dec Swap : [type a, type b, (a, b)!] (b, a)!
```

이렇게 작성하면 인자 목록 전체가 한 눈에 들어온다.

명시적 타입 대입자에는 `[type a: data]`와 같이 제약을 추가할 수도 있다. 이에 관한 내용은 [타입 제약](./constraints.md)에서 다룬다.

함수와 달리 전칭 타입은 순수 선형으로 고정되어 있지 않으며, 본문이 각각 `drop`이나 `share`를 만족할 경우 전칭 타입 자체도 `drop`이나 `share`를 만족할 수 있다.

## 생성

전칭 타입의 값은 함수처럼 생성하면 되지만, 인자가 일반 변수가 아니라 타입 변수이고 앞에 키워드 `type`을 붙여야 한다는 점에서 다르다.

위의 정의를 완성해 보자.

```par
dec None : [type a] Option<a>
def None = [type a] .none!

dec Swap : [type a, type b, (a, b)!] (b, a)!
def Swap = [type a, type b, pair]
  let (first, second)! = pair
  in (second, first)!
```

> 전칭 타입을 사용하다 보면 **선언부에 `type a`와 `type b`를 이미 작성했는데 왜 정의부에 한 번 더 작성해야 하느냐**는 불만을 가질 수 있다. 애초에 실제로 정의에서 사용하는 것 같지도 않으니 말이다. 하지만 그렇지 않다! `first`의 타입은 무엇인가? `a`이다. 그러면 `second`는? `b`이다. `type kek`과 `type dek`으로 작성했으면 각각 `kek`과 `dek`이 되었을 것이다. Par의 타입 검사기는 타입 이름을 마음대로 지어 내지 않는다.
>
> 추가로 다른 제네릭 함수를 호출하게 되어 타입 변수를 실제로 사용해야 하는 상황이 생긴다면, 멀리 갈 필요 없이 작성했던 이름을 바로 사용할 수 있다.

## 소멸

*전칭* 값을 사용하는 것도 함수 호출과 비슷하지만, 인자가 값이 아니라 구상 타입이고 앞에 키워드 `type`을 붙어야 한다는 점에서 다르다.

```par
def NoneInt = None(type Int)  // 타입이 `Option<Int>`로 추론됨

def Pair    = ("Hello!", 42)!
def Swapped = Swap(type String, type Int, Pair)
//          = (42, "Hello!")!
```
