# 재귀

*Par*의 여러 가지 설계 결정 중에서도 특히 야심찬 것은 다음과 같다.
- 타입 검사를 통해 무한루프를 방지하는 **전체성**
- 전역 타입 정의가 동의어에 불과한 **구조적 타입 시스템**

자기 참조 타입을 작성하고자 한다면, 전체성에 의해 다음 두 개념을 구분해야 한다.
- 유한한 값으로만 이루어진 *재귀 타입*
- 무한할 수도 있는 *쌍대재귀 타입*. Par에서는 [*반복 타입*](./iterative.md)이라고 한다.

구조적 타입 시스템을 채택함에 따라 자연스럽게 초보적인 자기 참조 타입 작성을 회피하고, 대신 익명의 자기 참조 타입을 작성할 수 있는 일급 문법을 지원하게 되었다.

이 점에서 Par는 매우 급진적이다. 일반적인 방법으로 단일 연결 리스트를 정의하려고 하면 컴파일이 되지 않는다.

```par
type IllegalList = either {
  .end!,
  .item(String) IllegalList,  // 오류! 순환 참조
}
```

일반적으로 **전역 정의로 이루어진 순환 참조는 금지된다.** 그 대신 다음 방법을 사용해야 한다.
- 익명의 자기 참조 타입 (재귀와 [반복](./iterative.md))
- 재귀를 다루는 단일 보편 구조 `begin`/`loop`. 하나의 문법으로 재귀적 소멸, 반복적 생성, [프로세스 문법](../process_syntax.md)의 명령형 스타일의 반복문을 모두 다룰 수 있다.

**이제 재귀 타입을 살펴보자!**

> **전체성이 강제된다고 해서 웹 서버나 게임을 구현할 수 없는 것은 *절대* 아니다.** 이런 동작은 보통 무한 이벤트 루프를 사용해 구현하지만, 굳이 그렇게 구현해야 하는 것은 아니며 Par에서 [반복](./iterative.md) 타입으로 지원하는 쌍대재귀를 사용할 수 있다.
>
> 아래의 Python 프로그램으로 구체적인 예시를 들어 보자.
>
> ```python
> def __main__():
>     while True:
>         req = next_request()
>         if req is None:
>             break
>         handle_request(req)
> ```
>
> 이 프로그램은 단순한 웹 서버로, 무한루프를 사용해 요청을 하나씩 처리한다.
>
> 무한루프를 사용하지 않도록 코드를 수정할 수 있을까? 물론이다!
>
> ```python
> class WebServer:
>     def close(self):
>         pass
>
>     def handle(req):
>         handle_request(req)
>
> def __main__():
>     start_server(WebServer())
> ```
>
> 작은 리팩토링으로 큰 결실을 얻을 수 있다. Par의 [반복](./iterative.md) 타입 역시 정확히 이 패턴을 구현하면서도 무한루프 버전의 편의성 역시 놓치지 않는다.

재귀(recursive) 타입은 키워드 `recursive` 다음에 `self`를 원하는 횟수만큼 포함하는 본문을 작성하면 된다. 이 `self`가 바로 자기 참조이다.

```par
type LegalList = recursive either {
  .end!,
  .item(String) self,  // 올바른 코드
}
```

> **(재귀나 [반복](./iterative.md)) 타입이 중첩되어 있다면, 여러 개 중 하나를 구분해야 할 수 있다.** 이때는 `recursive`와 `self`에 **레이블**을 추가할 수 있으며, 골뱅이표를 사용한다(`recursive@label`, `self@label`). 어떤 소문자 식별자든 레이블로 사용할 수 있다.

재귀 타입은 그 타입을 *전개*한 것과 같다고 볼 수 있다. 즉, 본문의 `self`를 원래의 재귀 타입으로 바꾸어도 된다.

1. 원래 정의:
   ```par
   recursive either {
     .end!,
     .item(String) self
   }
   ```
2. 1차 전개:
   ```par
   either {
     .end!,
     .item(String) recursive either {
       .end!,
       .item(String) self
     }
   }
   ```
3. 2차 전개:
   ```par
   either {
     .end!,
     .item(String) either {
       .end!,
       .item(String) recursive either {
         .end!,
         .item(String) self
       }
     }
   }
   ```
4. 등등...

> 재귀 타입의 본문은 분기로 시작하는 경우가 많지만, 반드시 그럴 필요는 없다. 아래와 같이 순서쌍으로 시작하는 비어 있지 않은 리스트를 예시로 들 수 있다.
>
> ```par
> type NonEmptyList<a> = recursive (a) either {
>   .end!,
>   .item self,
> }
> ```
>
> 분기로 시작하지 않는 재귀 타입의 다른 예시로는 유한한 스트림을 들 수 있다.
>
> ```par
> type FiniteStream<a> = recursive choice {
>   .close => !,
>   .next => either {
>     .end!,
>     .item(a) self,
>   }
> }
> ```
>
> 이 타입은 [선택](./choice.md) 타입으로 시작해 필요할 때 새로운 값을 폴링하거나, 남은 스트림을 닫을 수도 있다. 하지만 `FiniteStream<a>`는 재귀 타입이므로 직접 닫지 않을 경우 반드시 `.end!`에 도달하게 된다.
>
> 하지만 재귀 타입에는 중요한 제한이 있다. 무의미한 `self` 참조를 만들지 않기 위해서 **재귀 타입의 모든 `self` 참조는 분기로 감싸져 있어야 한다.** `either`가 `recursive` 바로 다음에 오지 않아도 되지만, `recursive`와 `self` 사이 *어딘가*에는 삽입되어 있어야 한다.
>
> ```par
> type ValidList<a> = recursive (a) either {
>   .end!,
>   .item self,  // 올바른 코드. `self`가 `either`로 감싸져 있다.
> }
> 
> type InvalidList<a> = recursive (a) self  // 오류! `self` 참조가 빠져나옴
> ```
>
> [반복](./iterative.md) 타입 역시 비슷한 제한이 있다. **반복 타입의 `self` 참조는 [선택](./choice.md)으로 감싸져 있어야 한다.**

*재귀 타입*의 핵심 기능은 **재귀 값은 유한**하며, **재귀 값에 대해 재귀할 수 있다**는 점이다.

## 생성

재귀 타입은 생성 문법이 따로 없으며, 재귀 타입이 전개된 것처럼 취급해서 내용물을 직접 생성하면 된다.

```par
type Tree = recursive either {
  .leaf Int,
  .node(self, self)!,
}

def SmallTree: Tree = .node(
  .node(
    .leaf 1,
    .leaf 2,
  )!,
  .node(
    .leaf 3,
    .leaf 4,
  )!,
)!
```

다른 재귀 값의 `self` 자리에 기존의 재귀 값을 사용할 수 있다.

```par
def BiggerTree: Tree = .node(SmallTree, SmallTree)!
```

자주 쓰이는 재귀 타입인 **리스트**는 표준 모듈에서 다음과 같이 정의하고 있다.

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

다음과 같이 생성하면 된다.

```par
dec OneThroughFive  : List<Int>
dec ZeroThroughFive : List<Int>

def OneThroughFive  = .item(1).item(2).item(3).item(4).item(5).end!
def ZeroThroughFive = .item(0) OneThroughFive
```

리스트는 특히 자주 쓰이기 때문에 더 간결하게 리스트를 생성할 수 있는 **문법 설탕** 역시 제공한다.

```par
def OneThroughFive = *(1, 2, 3, 4, 5)
```

단, 기존 리스트의 앞에 값을 덧붙이는 문법 설탕은 없으므로 `ZeroThroughFive`는 기존의 방법으로 생성해야 한다.

## 소멸

굳이 재귀가 필요 없다면 재귀 값을 소멸시킬 때도 재귀 타입이 전개된 것처럼 취급할 수 있다. 아래 예제에서는 `List<String>`을 기반이 되는 분기 값처럼 취급하고 있다.

```par
type Option<a> = either {
  .none!,
  .some a,
}

dec Head : [List<String>] Option<String>
def Head = [list] list.case {
  .end!      => .none!,
  .item(x) _ => .some x,
}
```

**재귀적 소멸에는 `.begin`/`.loop`를 사용한다.** 사용 방법은 다음과 같다.
1. `recursive` 타입의 값에 `.begin`을 적용한다.
2. 위에서 얻은 전개된 값에 원하는 연산을 적용한다.
3. *파생된* 재귀 값에 `.loop`를 적용한다. 이때 파생된 값이란 처음에 `.begin`을 적용한 값의 `self`에 해당하는 값을 말한다.

정수의 리스트를 입력받아 합을 구하는 함수를 만들면서 실제 용례를 확인해 보자.

0. 재귀 타입 (`List<Int>`)의 값 (`list`)를 입력받는다.
   ```par
   dec SumList : [List<Int>] Int

   def SumList = [list]
   ```
1. 입력받은 값에 `.begin`을 적용한다.
   ```par
                        list.begin
   ```
2. 해당하는 값의 모든 선지에 대해 분기한다.
   ```par
                                  .case {
     .end!       => 0,
     .item(x) xs =>
   ```
   리스트가 비어 있을 경우에는 합이 `0`이 된다. 그렇지 않으면 정수 `x`에
   ```par
                    x +
   ```
   리스트 나머지 부분 `xs`의 합을 더해야 한다.
4. `xs`가 처음에 `.begin`을 적용한 `list`에서 *파생된* 값이므로, `.loop`를 사용해 `xs`의 합을 재귀적으로 구할 수 있다.
   ```par
                               xs.loop
   ```
   이제 중괄호를 닫으면 된다.
   ```par
   }
   ```

완성된 코드는 다음과 같다.

```par
def SumList = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
}
```

`.loop`의 동작은 대응하는 `.begin`으로 돌아가되, 새롭게 얻은 값을 사용하는 것으로 생각할 수 있다.

`.begin`/`.loop`의 의미를 이해하는 데도 재귀 타입 자체와 같이 *전개*를 사용하는 것이 적합하다. **`.loop`를 보이는 대로 `.begin`부터 시작하는 본문으로 계속 치환해도 `.begin`/`.loop`의 의미는 변하지 않는다.**

직접 확인해 보자.
1. 원래 코드:
   ```par
   def SumList = [list] list.begin.case {
     .end!       => 0,
     .item(x) xs => x + xs.loop,
   }
   ```
2. 1차 전개:
   ```par
   def SumList = [list] list.case {
     .end!       => 0,
     .item(x) xs => x + xs.begin.case {
       .end!       => 0,
       .item(x) xs => x + xs.loop,
     },
   }
   ```
3. 2차 전개:
   ```par
   def SumList = [list] list.case {
     .end!       => 0,
     .item(x) xs => x + xs.case {
       .end!       => 0,
       .item(x) xs => x + xs.begin.case {
         .end!       => 0,
         .item(x) xs => x + xs.loop,
       },
     },
   }
   ```
4. 등등...

여러 개의 파생 값에 같은 `.loop`를 적용해도 된다. 아래는 위에서 정의한 `Tree` 타입의 단말 노드를 모두 더하는 함수이다.

```par
dec SumTree : [Tree] Int
def SumTree = [tree] tree.begin.case {
  .leaf number        => number,
  .node(left, right)! => {left.loop} + {right.loop},
}

def BiggerSum = SumTree(BiggerTree)  // = 20
```

> **여러 개의 `.begin`/`.loop`가 중첩되어 있다면, 무슨 `.begin`의 `.loop`인지를 구분할 필요가 있다.** 타입과 마찬가지로 여기서도 레이블을 사용해 `.begin@label`과 `.loop@label`이라고 작성하면 된다.
>
> TODO:
> ```par
> type Tree<a> = recursive List<(a) self>
> ```

### 지역 변수의 유지

잠시 다른 언어로 넘어가, 하스켈에서 리스트의 각 원소를 정해진 값만큼 증가시키는 함수를 작성해 보자.

```haskell
incBy n []     = []
incBy n (x:xs) = (x + n) : incBy n xs
```

위의 재귀 함수에는 리스트 전체에 걸쳐 기억해야 하는 매개변수 `n`이 있다. 하스켈에서는 해당 값을 재귀 호출에 명시적으로 전달하고 있다.

이제 *Par*의 경우를 살펴 보자. Par의 `.loop`에는 **지역 변수가 알아서 다음 반복으로 전달된다**는 편리한 기능이 있다.

```par
dec IncBy : [List<Int>, Int] List<Int>
def IncBy = [list, n] list.begin.case {
  .end!       => .end!,
  .item(x) xs => .item(x + n) xs.loop,
}
```

`xs.loop`에서는 증가값인 `n`을 전혀 언급하고 있지 않지만, 해당 값이 자동으로 전달되기 때문에 재귀 호출 전체에서 유지되고 있다.

`begin`/`loop` 문법이 그저 보편 재귀 구조를 넘어서 일반적인 재귀와 명령형 루프의 장점을 모두 취하게 된 비결이 바로 이것이다.

> 지역 변수가 왜, 어떻게 유지되는 것인지 헷갈린다면, 위 함수에서 `.begin`/`.loop`를 전개해 보자. `.loop`를 반복되는 본문으로 치환하면 치환한 식에서도 `n`이 범위 안에 있는 것을 확인할 수 있다. `.begin`/`.loop`를 전개해도 정말 의미가 변하지 않았다.

`.begin`/`.loop`를 식의 깊은 곳에서도 사용할 수 있다는 점을 생각하면, 지역 변수가 유지되는 덕에 별도의 헬퍼 함수를 쓰지 않아도 된다는 장점 역시 있다.

다시 하스켈로 돌아가 리스트를 뒤집는 다음 함수를 살펴 보자.

```haskell
reverse list = reverseHelper [] list

reverseHelper acc []     = acc
reverseHelper acc (x:xs) = reverseHelper (x:acc) xs
```

이 함수는 누산값 `acc`를 상태로 사용하고, 매번 반복할 때마다 누산값의 맨 앞에 원소를 하나씩 추가하면서 리스트를 뒤집는다. 하스켈에서는 이 연산에 헬퍼 재귀 함수가 필요하다.

Par에서는 필요 없다!

```par
dec Reverse : [type a, List<a>] List<a>
def Reverse = [type a, list]
  let acc: List<a> = .end!
  in list.begin.case {
    .end!       => acc,
    .item(x) xs => let acc = .item(x) acc in xs.loop,
  }

def TestReverse = Reverse(type Int, *(1, 2, 3, 4, 5))  // = *(5, 4, 3, 2, 1)
```

`acc`에 새로운 값을 재대입하고 `xs.loop`로 반복하면 간단히 구현할 수 있다.

### 전체성의 임시방편, `.unfounded`

프로그램에 있는 재귀 함수가 전체 함수(모든 입력에 대해 값을 내놓음)임을 알 수 있는데도 Par의 타입 검사기가 함수를 거부할 경우, `.begin`을 `.unfounded`로 바꾸면 전체성 확인을 비활성화할 수 있다.

현재 Par의 전체성 검사기는 특히 분할 정복법 등 일부 알고리즘을 검사하지 못하며, 재귀 알고리즘을 여러 함수로 분리하는 경우에도 취약하기 때문에 이때 `.unfounded`를 사용하는 것은 괜찮다. 하지만 타입 시스템을 더욱 개선해서 최종적으로는 `.unfounded`를 언어에서 제거하는 것이 장기 목표이다.
